//! The `@page` descriptors this engine takes (Paged Media 3 §7.1 and
//! §6; M20): `size`, and `margin` with its longhands. Each parser takes
//! a declaration's value as the cascade has it and says `None` when the
//! value is not one the descriptor takes, so that it is dropped.

use css::Token;

use super::{LengthBases, parse_length_percentage_or_auto, parse_margin_shorthand};
use super::{parse_non_negative_length_px, split_whitespace_groups};
use crate::computed::LengthPercentageAuto;
use crate::page::{Orientation, PageSize, named_page_size};

/// `size`: `auto`, one or two non-negative lengths, or a named size and
/// an orientation, either or both, in either order.
pub(super) fn parse_size(value: &[Token], bases: &LengthBases<'_>) -> Option<PageSize> {
    let groups = split_whitespace_groups(value);
    let ident = |group: &[Token]| match group {
        [Token::Ident(word)] => Some(word.to_ascii_lowercase()),
        _ => None,
    };
    if let [only] = groups.as_slice()
        && ident(only).as_deref() == Some("auto")
    {
        return Some(PageSize::Auto);
    }
    // Lengths.
    let lengths: Option<Vec<f32>> = groups
        .iter()
        .map(|g| parse_non_negative_length_px(g, bases))
        .collect();
    match lengths.as_deref() {
        Some([side]) => {
            return Some(PageSize::Exact {
                width: *side,
                height: *side,
            });
        }
        Some([width, height]) => {
            return Some(PageSize::Exact {
                width: *width,
                height: *height,
            });
        }
        _ => {}
    }
    // `<page-size> || [ portrait | landscape ]`.
    let mut size = None;
    let mut orientation = None;
    if groups.is_empty() || groups.len() > 2 {
        return None;
    }
    for group in &groups {
        let word = ident(group)?;
        match word.as_str() {
            "portrait" | "landscape" if orientation.is_none() => {
                orientation = Some(if word == "portrait" {
                    Orientation::Portrait
                } else {
                    Orientation::Landscape
                });
            }
            _ if size.is_none() => size = Some(named_page_size(&word)?),
            _ => return None,
        }
    }
    Some(match (size, orientation) {
        (Some(size), orientation) => {
            let (width, height) = orientation.unwrap_or(Orientation::Portrait).turn(size);
            PageSize::Exact { width, height }
        }
        (None, Some(orientation)) => PageSize::Oriented(orientation),
        (None, None) => return None,
    })
}

/// `margin` on a page: one to four lengths, percentages or `auto`.
pub(super) fn parse_page_margin(
    value: &[Token],
    bases: &LengthBases<'_>,
) -> Option<[LengthPercentageAuto; 4]> {
    parse_margin_shorthand(value, bases)
}

/// A margin longhand on a page.
pub(super) fn parse_page_margin_side(
    value: &[Token],
    bases: &LengthBases<'_>,
) -> Option<LengthPercentageAuto> {
    parse_length_percentage_or_auto(value, bases)
}
