//! The values of the break properties, `orphans`, `widows` and
//! `print-color-adjust` (css-break-3 §3, css-color-adjust-1 §3.3; M20),
//! and of the legacy `page-break-*` properties, which css-break-3 §3.4
//! makes shorthands of the break properties: CSS2's values, `always`
//! being `page`. Each parser takes a declaration's value as the cascade
//! has it, trimmed and without `!important`, and says `None` when the
//! value is not one the property takes.

use css::Token;

use super::single_ident;
use crate::computed::{BreakBetween, BreakInside, PrintColorAdjust};

/// `break-before` and `break-after`.
pub(super) fn parse_break_between(value: &[Token]) -> Option<BreakBetween> {
    Some(match single_ident(value)?.to_ascii_lowercase().as_str() {
        "auto" => BreakBetween::Auto,
        "avoid" => BreakBetween::Avoid,
        "avoid-page" => BreakBetween::AvoidPage,
        "page" => BreakBetween::Page,
        "left" => BreakBetween::Left,
        "right" => BreakBetween::Right,
        "recto" => BreakBetween::Recto,
        "verso" => BreakBetween::Verso,
        "avoid-column" => BreakBetween::AvoidColumn,
        "column" => BreakBetween::Column,
        "avoid-region" => BreakBetween::AvoidRegion,
        "region" => BreakBetween::Region,
        _ => return None,
    })
}

/// `page-break-before` and `page-break-after`: `auto`, `always`, `avoid`,
/// `left` and `right`.
pub(super) fn parse_page_break_between(value: &[Token]) -> Option<BreakBetween> {
    Some(match single_ident(value)?.to_ascii_lowercase().as_str() {
        "auto" => BreakBetween::Auto,
        "always" => BreakBetween::Page,
        "avoid" => BreakBetween::Avoid,
        "left" => BreakBetween::Left,
        "right" => BreakBetween::Right,
        _ => return None,
    })
}

/// `break-inside`.
pub(super) fn parse_break_inside(value: &[Token]) -> Option<BreakInside> {
    Some(match single_ident(value)?.to_ascii_lowercase().as_str() {
        "auto" => BreakInside::Auto,
        "avoid" => BreakInside::Avoid,
        "avoid-page" => BreakInside::AvoidPage,
        "avoid-column" => BreakInside::AvoidColumn,
        "avoid-region" => BreakInside::AvoidRegion,
        _ => return None,
    })
}

/// `page-break-inside`: `auto` and `avoid`.
pub(super) fn parse_page_break_inside(value: &[Token]) -> Option<BreakInside> {
    Some(match single_ident(value)?.to_ascii_lowercase().as_str() {
        "auto" => BreakInside::Auto,
        "avoid" => BreakInside::Avoid,
        _ => return None,
    })
}

/// `orphans` and `widows`: an integer of one or more.
pub(super) fn parse_line_count(value: &[Token]) -> Option<u32> {
    match value {
        [
            Token::Number {
                value,
                is_integer: true,
            },
        ] if *value >= 1.0 => Some(value.min(u32::MAX as f64) as u32),
        _ => None,
    }
}

/// `print-color-adjust`.
pub(super) fn parse_print_color_adjust(value: &[Token]) -> Option<PrintColorAdjust> {
    Some(match single_ident(value)?.to_ascii_lowercase().as_str() {
        "economy" => PrintColorAdjust::Economy,
        "exact" => PrintColorAdjust::Exact,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        css::tokenize(source)
    }

    #[test]
    fn the_break_values_are_fragmentation_3s() {
        for (source, want) in [
            ("auto", BreakBetween::Auto),
            ("AVOID", BreakBetween::Avoid),
            ("avoid-page", BreakBetween::AvoidPage),
            ("page", BreakBetween::Page),
            ("left", BreakBetween::Left),
            ("right", BreakBetween::Right),
            ("recto", BreakBetween::Recto),
            ("verso", BreakBetween::Verso),
            ("avoid-column", BreakBetween::AvoidColumn),
            ("column", BreakBetween::Column),
            ("avoid-region", BreakBetween::AvoidRegion),
            ("region", BreakBetween::Region),
        ] {
            assert_eq!(parse_break_between(&tokens(source)), Some(want), "{source}");
        }
        // Level 4's, CSS2's and nonsense.
        for source in ["always", "all", "none", "page page", "avoid region", "1"] {
            assert_eq!(parse_break_between(&tokens(source)), None, "{source}");
        }
        for (source, want) in [
            ("auto", BreakInside::Auto),
            ("avoid", BreakInside::Avoid),
            ("avoid-page", BreakInside::AvoidPage),
            ("avoid-column", BreakInside::AvoidColumn),
            ("avoid-region", BreakInside::AvoidRegion),
        ] {
            assert_eq!(parse_break_inside(&tokens(source)), Some(want), "{source}");
        }
        for source in ["page", "region", "auto avoid"] {
            assert_eq!(parse_break_inside(&tokens(source)), None, "{source}");
        }
    }

    #[test]
    fn the_legacy_page_breaks_take_css2s_values() {
        for (source, want) in [
            ("auto", BreakBetween::Auto),
            ("always", BreakBetween::Page),
            ("Always", BreakBetween::Page),
            ("avoid", BreakBetween::Avoid),
            ("left", BreakBetween::Left),
            ("right", BreakBetween::Right),
        ] {
            assert_eq!(
                parse_page_break_between(&tokens(source)),
                Some(want),
                "{source}"
            );
        }
        for source in ["page", "recto", "verso", "column", "avoid-page"] {
            assert_eq!(parse_page_break_between(&tokens(source)), None, "{source}");
        }
        assert_eq!(
            parse_page_break_inside(&tokens("avoid")),
            Some(BreakInside::Avoid)
        );
        assert_eq!(parse_page_break_inside(&tokens("avoid-page")), None);
    }

    #[test]
    fn line_counts_are_positive_integers() {
        assert_eq!(parse_line_count(&tokens("1")), Some(1));
        assert_eq!(parse_line_count(&tokens("234")), Some(234));
        assert_eq!(
            parse_line_count(&tokens("1e20")),
            None,
            "not an integer token"
        );
        assert_eq!(parse_line_count(&tokens("99999999999")), Some(u32::MAX));
        for source in ["0", "-1", "2.5", "auto", "1 2"] {
            assert_eq!(parse_line_count(&tokens(source)), None, "{source}");
        }
        assert_eq!(
            parse_print_color_adjust(&tokens("EXACT")),
            Some(PrintColorAdjust::Exact)
        );
        assert_eq!(parse_print_color_adjust(&tokens("auto")), None);
    }
}
