//! The page box (M20): what `@page` rules (CSS Paged Media 3) say of a
//! page, and the selectors that say which pages they are for. See
//! `docs/design/media.md`, "Print: the page box and `@page`".
//!
//! The rules are taken from the author's sheets inside the conditions
//! and layers their sheets put them in, and cascaded for one page by
//! [`crate::Stylesheets::page_style`] as style rules are for an element,
//! the page selectors' specificity in place of a selector's. Of their
//! descriptors, `size` and `margin` with its longhands are taken; the
//! page-margin boxes inside a rule are dropped by the css crate's
//! parser, which drops every at-rule inside a declaration block.

use css::Token;

use crate::computed::LengthPercentageAuto;
use crate::media::{split_commas, trim};

/// Where a page stands among the document's pages: what the page
/// selectors `:first`, `:left`, `:right` and `:blank` ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PagePosition {
    /// The document's first page.
    pub first: bool,
    /// A left page; a right one when not. The document being left to
    /// right, its first page is a right page.
    pub left: bool,
    /// A page a forced break to a left or right page left empty.
    pub blank: bool,
}

/// What `size` gives the page box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PageSize {
    /// `auto`, or no rule setting it: the size the print is asked at.
    #[default]
    Auto,
    /// A width and a height, in CSS pixels: one or two lengths, or a
    /// named size in an orientation.
    Exact { width: f32, height: f32 },
    /// `portrait` or `landscape` alone: the size the print is asked at,
    /// turned to it.
    Oriented(Orientation),
}

/// A page's orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Taller than wide, or square.
    Portrait,
    /// Wider than tall.
    Landscape,
}

impl PageSize {
    /// The page box's width and height, `default` being the size the
    /// print is asked at.
    pub fn resolve(self, default: (f32, f32)) -> (f32, f32) {
        match self {
            PageSize::Auto => default,
            PageSize::Exact { width, height } => (width, height),
            PageSize::Oriented(orientation) => orientation.turn(default),
        }
    }
}

impl Orientation {
    /// `(width, height)` turned to this orientation.
    pub fn turn(self, (width, height): (f32, f32)) -> (f32, f32) {
        let (short, long) = if width <= height {
            (width, height)
        } else {
            (height, width)
        };
        match self {
            Orientation::Portrait => (short, long),
            Orientation::Landscape => (long, short),
        }
    }
}

/// What the `@page` rules say of one page.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageStyle {
    /// `size`.
    pub size: PageSize,
    /// The margins, top, right, bottom and left; `None` where no rule
    /// sets one, which leaves it to the print's own. A percentage is of
    /// the page box's width for the left and right margins and of its
    /// height for the top and bottom (Paged Media 3 §6).
    pub margin: [Option<LengthPercentageAuto>; 4],
}

/// US Letter, 8.5 by 11 inches, in CSS pixels: the page the reference
/// browser prints to PDF on when nothing says otherwise.
pub const LETTER: (f32, f32) = (8.5 * IN, 11.0 * IN);

/// The margin the reference browser gives each side of a page no rule
/// sets one for, measured from its print to PDF: 28 points.
pub const DEFAULT_MARGIN: f32 = 28.0 * IN / 72.0;

/// A page box: its size, and its margins, top, right, bottom and left,
/// in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageBox {
    pub width: f32,
    pub height: f32,
    pub margin: [f32; 4],
}

impl PageBox {
    /// The page area's width and height: the page box less its margins,
    /// at least a pixel each way.
    pub fn area(&self) -> (f32, f32) {
        let [top, right, bottom, left] = self.margin;
        (
            (self.width - left - right).max(1.0),
            (self.height - top - bottom).max(1.0),
        )
    }

    /// Where the page area's top left corner is in the page box: its
    /// left and top margins, rounded to whole CSS pixels as the reference
    /// browser places it.
    pub fn origin(&self) -> (f32, f32) {
        (self.margin[3].round(), self.margin[0].round())
    }

    /// The page area the document is laid out in and cut into pages by:
    /// the area's width and height rounded up to whole CSS pixels, as
    /// the reference browser lays a page out. On Letter at the default
    /// margins it lays the document out 742 pixels wide and fits 982
    /// pixels of it on each page, the area's edge cutting off the last
    /// two thirds of a pixel.
    pub fn layout_area(&self) -> (f32, f32) {
        let (width, height) = self.area();
        (width.ceil(), height.ceil())
    }

    /// The page area as the viewport units measure it: the area's width
    /// and height rounded down to whole CSS pixels, the reference
    /// browser's viewport, 741 by 981 pixels on Letter at the default
    /// margins.
    pub fn viewport(&self) -> (f32, f32) {
        let (width, height) = self.area();
        (width.floor(), height.floor())
    }
}

impl PageStyle {
    /// The page box these say, `default` being the size the print is
    /// asked at and `default_margin` the margin of a side no rule sets:
    /// `auto` is zero, as in the reference browser, and a percentage is
    /// of the page box's width for the left and right margins and of its
    /// height for the top and bottom (Paged Media 3 §6).
    pub fn page_box(&self, default: (f32, f32), default_margin: f32) -> PageBox {
        let (width, height) = self.size.resolve(default);
        let mut margin = [default_margin; 4];
        for (side, value) in self.margin.iter().enumerate() {
            let basis = if side % 2 == 0 { height } else { width };
            if let Some(value) = value {
                margin[side] = value.resolve(basis, 0.0);
            }
        }
        PageBox {
            width,
            height,
            margin,
        }
    }
}

/// The named page sizes of Paged Media 3 §7.1.1, by their names in
/// ASCII lowercase, in millimetres or inches as the draft gives them,
/// portrait.
const PAGE_SIZES: [(&str, f32, f32); 10] = [
    ("a5", 148.0 * MM, 210.0 * MM),
    ("a4", 210.0 * MM, 297.0 * MM),
    ("a3", 297.0 * MM, 420.0 * MM),
    ("b5", 176.0 * MM, 250.0 * MM),
    ("b4", 250.0 * MM, 353.0 * MM),
    ("jis-b5", 182.0 * MM, 257.0 * MM),
    ("jis-b4", 257.0 * MM, 364.0 * MM),
    ("letter", 8.5 * IN, 11.0 * IN),
    ("legal", 8.5 * IN, 14.0 * IN),
    ("ledger", 11.0 * IN, 17.0 * IN),
];

const IN: f32 = 96.0;
const MM: f32 = 96.0 / 25.4;

/// The named page size `name`, any case, portrait, in CSS pixels.
pub fn named_page_size(name: &str) -> Option<(f32, f32)> {
    PAGE_SIZES
        .iter()
        .find(|(known, ..)| name.eq_ignore_ascii_case(known))
        .map(|&(_, width, height)| (width, height))
}

/// One page selector: an optional page type name and the pseudo-pages
/// after it, with nothing between them (Paged Media 3 §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PageSelector {
    /// A page type name, which no page here has: there is no `page`
    /// property, so a selector with one matches nothing.
    named: bool,
    first: bool,
    blank: bool,
    left: bool,
    right: bool,
    /// The count of `:first` and `:blank`, and of `:left` and `:right`.
    counts: (u32, u32),
}

/// The specificity of a page selector (Paged Media 3 §4.4): whether it
/// names a page type, how many `:first` and `:blank` it has, and how
/// many `:left` and `:right`.
pub(crate) type PageSpecificity = (u32, u32, u32);

impl PageSelector {
    /// Whether the selector matches a page at `position`.
    pub(crate) fn matches(&self, position: PagePosition) -> bool {
        !self.named
            && (!self.first || position.first)
            && (!self.blank || position.blank)
            && (!self.left || position.left)
            && (!self.right || !position.left)
    }

    pub(crate) fn specificity(&self) -> PageSpecificity {
        (u32::from(self.named), self.counts.0, self.counts.1)
    }
}

/// The selectors of an `@page` rule's prelude: none written is one that
/// matches every page; `None` when the prelude is not a
/// `<page-selector-list>`, which drops the rule.
pub(crate) fn parse_page_selectors(prelude: &[Token]) -> Option<Vec<PageSelector>> {
    if trim(prelude).is_empty() {
        return Some(vec![PageSelector::default()]);
    }
    split_commas(prelude)
        .into_iter()
        .map(|selector| parse_page_selector(trim(selector)))
        .collect()
}

/// `[ <ident-token>? <pseudo-page>* ]!`, no white space inside.
fn parse_page_selector(tokens: &[Token]) -> Option<PageSelector> {
    let mut selector = PageSelector::default();
    let mut rest = tokens;
    if let [Token::Ident(_), after @ ..] = rest {
        selector.named = true;
        rest = after;
    }
    while let [Token::Colon, Token::Ident(name), after @ ..] = rest {
        match name.to_ascii_lowercase().as_str() {
            "first" => {
                selector.first = true;
                selector.counts.0 += 1;
            }
            "blank" => {
                selector.blank = true;
                selector.counts.0 += 1;
            }
            "left" => {
                selector.left = true;
                selector.counts.1 += 1;
            }
            "right" => {
                selector.right = true;
                selector.counts.1 += 1;
            }
            _ => return None,
        }
        rest = after;
    }
    let empty = selector == PageSelector::default();
    (rest.is_empty() && !empty).then_some(selector)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selectors(source: &str) -> Option<Vec<PageSelector>> {
        parse_page_selectors(&css::tokenize(source))
    }

    fn at(first: bool, left: bool, blank: bool) -> PagePosition {
        PagePosition { first, left, blank }
    }

    #[test]
    fn page_selectors_parse_by_the_draft() {
        for valid in [
            "",
            "  ",
            ":first",
            ":FIRST",
            ":left",
            ":right",
            ":blank",
            ":first:left",
            ":first, :left",
            "a",
            "page1",
            "name1, name2",
            "name1\\{\\}name2",
            "foo:first",
        ] {
            assert!(selectors(valid).is_some(), "{valid:?}");
        }
        for invalid in [
            ", ",
            "a,",
            ",a",
            ": first",
            "foo :first",
            ":left :right",
            ":foo",
            ":first()",
            ":nth(1)",
            "a b",
            "1",
        ] {
            assert_eq!(selectors(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn page_selectors_match_by_position_and_count_by_kind() {
        let one = |source: &str| selectors(source).expect("valid").remove(0);
        let first_right = at(true, false, false);
        let left = at(false, true, false);
        let blank_right = at(false, false, true);
        assert!(selectors("").unwrap()[0].matches(left));
        assert!(one(":first").matches(first_right) && !one(":first").matches(left));
        assert!(one(":right").matches(first_right) && !one(":right").matches(left));
        assert!(one(":left").matches(left) && !one(":left").matches(blank_right));
        assert!(one(":blank").matches(blank_right) && !one(":blank").matches(left));
        assert!(one(":first:right").matches(first_right));
        assert!(!one(":first:left").matches(first_right));
        assert!(!one(":left:right").matches(left), "never both");
        assert!(!one("cover").matches(first_right), "no page has a name");
        assert_eq!(one(":first").specificity(), (0, 1, 0));
        assert_eq!(one(":blank:left").specificity(), (0, 1, 1));
        assert_eq!(one("cover:first").specificity(), (1, 1, 0));
        assert_eq!(selectors("").unwrap()[0].specificity(), (0, 0, 0));
    }

    #[test]
    fn page_boxes_take_their_defaults_and_resolve_their_margins() {
        let none = PageStyle::default().page_box(LETTER, DEFAULT_MARGIN);
        assert_eq!((none.width, none.height), (816.0, 1056.0));
        let (w, h) = none.area();
        // The reference browser's page area on Letter: 741.33 by 981.33.
        assert!((w - 741.333).abs() < 0.01 && (h - 981.333).abs() < 0.01);
        assert_eq!(none.origin(), (37.0, 37.0));
        assert_eq!(none.layout_area(), (742.0, 982.0));
        assert_eq!(none.viewport(), (741.0, 981.0));
        // An area of 740.4 by 980.4, as the reference browser rounds it.
        let fractional = PageStyle {
            margin: [Some(LengthPercentageAuto::Px(37.8)); 4],
            ..PageStyle::default()
        }
        .page_box(LETTER, DEFAULT_MARGIN);
        assert_eq!(fractional.origin(), (38.0, 38.0));
        assert_eq!(fractional.layout_area(), (741.0, 981.0));
        assert_eq!(fractional.viewport(), (740.0, 980.0));
        let style = PageStyle {
            size: PageSize::Oriented(Orientation::Landscape),
            margin: [
                Some(LengthPercentageAuto::Percent(10.0)),
                Some(LengthPercentageAuto::Auto),
                None,
                Some(LengthPercentageAuto::Px(-20.0)),
            ],
        };
        let b = style.page_box(LETTER, DEFAULT_MARGIN);
        assert_eq!((b.width, b.height), (1056.0, 816.0));
        assert_eq!(b.margin[0], 81.6, "10% of the height");
        assert_eq!(b.margin[1], 0.0, "auto");
        assert_eq!(b.margin[2], DEFAULT_MARGIN);
        assert_eq!(b.margin[3], -20.0);
        assert_eq!(b.origin(), (-20.0, 82.0));
        // Margins past the page leave a pixel.
        let wide = PageStyle {
            margin: [Some(LengthPercentageAuto::Px(1000.0)); 4],
            ..PageStyle::default()
        };
        assert_eq!(wide.page_box(LETTER, 0.0).area(), (1.0, 1.0));
        assert_eq!(wide.page_box(LETTER, 0.0).layout_area(), (1.0, 1.0));
        assert_eq!(wide.page_box(LETTER, 0.0).viewport(), (1.0, 1.0));
    }

    #[test]
    fn sizes_resolve_against_the_size_asked_for() {
        let letter = named_page_size("LETTER").unwrap();
        assert_eq!(letter, (816.0, 1056.0));
        let a4 = named_page_size("a4").unwrap();
        assert!((a4.0 - 793.7).abs() < 0.1 && (a4.1 - 1122.5).abs() < 0.1);
        assert_eq!(named_page_size("a0"), None);
        assert_eq!(PageSize::Auto.resolve(letter), letter);
        assert_eq!(
            PageSize::Oriented(Orientation::Landscape).resolve(letter),
            (1056.0, 816.0)
        );
        assert_eq!(
            PageSize::Oriented(Orientation::Portrait).resolve((1056.0, 816.0)),
            letter
        );
        assert_eq!(
            PageSize::Exact {
                width: 10.0,
                height: 20.0
            }
            .resolve(letter),
            (10.0, 20.0)
        );
    }
}
