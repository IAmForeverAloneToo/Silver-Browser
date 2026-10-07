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
