//! The flow-relative properties (CSS Logical Properties 1, D74): each
//! a name for a physical property that the element's `direction` picks,
//! in horizontal writing, the only mode (vertical writing modes are not
//! in this batch, its design note's section 2): the block axis's start
//! and end are the top and the bottom, the inline axis's the left and
//! the right, swapped by `direction: rtl`. A longhand sets its physical
//! longhand; a two-value shorthand (`margin-inline`) its start and end
//! physical longhands, one value both; `border-inline` both sides' whole
//! border. The cascade applies them where they stand in its order, so
//! that a later physical declaration wins over an earlier logical one
//! and the other way round, after `direction` is known.

use crate::computed::{BOTTOM, Direction, LEFT, RIGHT, TOP};

/// What a flow-relative property sets, by physical names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mapped {
    /// One property, the whole value its.
    One(String),
    /// A shorthand of a start and an end: one value for both, or two,
    /// the first the start's.
    Pair([String; 2]),
    /// A shorthand both of whose sides take the whole value.
    Both([String; 2]),
}

/// The sides' names, by [`TOP`], [`RIGHT`], [`BOTTOM`] and [`LEFT`].
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// The physical side of an axis's start or end under `direction`.
fn side(axis: &str, end: &str, direction: Direction) -> Option<usize> {
    let rtl = direction == Direction::Rtl;
    Some(match (axis, end) {
        ("block", "start") => TOP,
        ("block", "end") => BOTTOM,
        ("inline", "start") if rtl => RIGHT,
        ("inline", "start") => LEFT,
        ("inline", "end") if rtl => LEFT,
        ("inline", "end") => RIGHT,
        _ => return None,
    })
}

/// The physical names of the start and the end of `axis`.
fn ends(axis: &str, direction: Direction) -> Option<[usize; 2]> {
    Some([
        side(axis, "start", direction)?,
        side(axis, "end", direction)?,
    ])
}

/// The prefixes of the box properties with flow-relative sides: their
/// physical sides are `<prefix>-<side>`, but for `inset`'s, which are
/// the sides' names alone.
const BOX: [&str; 6] = [
    "margin",
    "padding",
    "inset",
    "border",
    "scroll-margin",
    "scroll-padding",
];

/// What the flow-relative property `name` sets under `direction`; `None`
/// for a property that is not one.
pub(crate) fn map(name: &str, direction: Direction) -> Option<Mapped> {
    let one = |physical: &str| Some(Mapped::One(physical.to_owned()));
    match name {
        "inline-size" => return one("width"),
        "block-size" => return one("height"),
        "min-inline-size" => return one("min-width"),
        "min-block-size" => return one("min-height"),
        "max-inline-size" => return one("max-width"),
        "max-block-size" => return one("max-height"),
        "overflow-inline" => return one("overflow-x"),
        "overflow-block" => return one("overflow-y"),
        _ => {}
    }
    // `border-<block>-<inline>-radius`, a corner by its two sides.
    if let Some(corner) = name
        .strip_prefix("border-")
        .and_then(|rest| rest.strip_suffix("-radius"))
        && let Some((block, inline)) = corner.split_once('-')
        && matches!(block, "start" | "end")
    {
        let vertical = SIDES[side("block", block, direction)?];
        let horizontal = SIDES[side("inline", inline, direction)?];
        return one(&format!("border-{vertical}-{horizontal}-radius"));
    }
    let (prefix, rest) = BOX.iter().find_map(|prefix| {
        let rest = name.strip_prefix(prefix)?.strip_prefix('-')?;
        Some((*prefix, rest))
    })?;
    let physical = |side: usize, suffix: &str| -> String {
        let side = SIDES[side];
        match (prefix, suffix) {
            ("inset", _) => side.to_owned(),
            (_, "") => format!("{prefix}-{side}"),
            _ => format!("{prefix}-{side}-{suffix}"),
        }
    };
    let parts: Vec<&str> = rest.split('-').collect();
    let border_part =
        |part: &str| prefix == "border" && matches!(part, "width" | "style" | "color");
    match parts[..] {
        // `margin-inline`, `border-block`: the axis's two sides.
        [axis] => {
            let [start, end] = ends(axis, direction)?;
            let sides = [physical(start, ""), physical(end, "")];
            Some(if prefix == "border" {
                Mapped::Both(sides)
            } else {
                Mapped::Pair(sides)
            })
        }
        // `margin-inline-start`, `border-block-end`: one side.
        [axis, end @ ("start" | "end")] => one(&physical(side(axis, end, direction)?, "")),
        // `border-inline-width`: the axis's two sides' widths.
        [axis, part] if border_part(part) => {
            let [start, end] = ends(axis, direction)?;
            Some(Mapped::Pair([physical(start, part), physical(end, part)]))
        }
        // `border-block-start-color`: one side's colour.
        [axis, end @ ("start" | "end"), part] if border_part(part) => {
            one(&physical(side(axis, end, direction)?, part))
        }
        _ => None,
    }
}

/// The physical longhand the flow-relative longhand `name` is under
/// `direction`; `None` for any other property, a shorthand among them.
pub(crate) fn physical_longhand(name: &str, direction: Direction) -> Option<String> {
    match map(name, direction)? {
        Mapped::One(physical) if !is_shorthand(&physical) => Some(physical),
        _ => None,
    }
}

/// Whether a physical name [`map`] gives is a shorthand: a border side's.
fn is_shorthand(physical: &str) -> bool {
    SIDES
        .iter()
        .any(|side| physical.strip_prefix("border-") == Some(side))
}

/// The flow-relative longhands of the shorthand `name`, in order, if it
/// is one: a two-value shorthand's start and end, a border side's width,
/// style and colour, `border-inline`'s and `border-block`'s both sides'.
pub fn longhands_of(name: &str) -> Option<Vec<String>> {
    if map(name, Direction::Ltr).is_none() || physical_longhand(name, Direction::Ltr).is_some() {
        return None;
    }
    let border = name.starts_with("border-");
    let parts: Vec<&str> = name.split('-').collect();
    let axis_at = parts
        .iter()
        .position(|p| matches!(*p, "inline" | "block"))?;
    let prefix = parts[..axis_at].join("-");
    let axis = parts[axis_at];
    let rest = &parts[axis_at + 1..];
    let side_parts = |end: &str| -> Vec<String> {
        ["width", "style", "color"]
            .iter()
            .map(|part| format!("{prefix}-{axis}-{end}-{part}"))
            .collect()
    };
    Some(match rest {
        [] if border => {
            let mut all = side_parts("start");
            all.extend(side_parts("end"));
            all
        }
        [] => vec![
            format!("{prefix}-{axis}-start"),
            format!("{prefix}-{axis}-end"),
        ],
        [end @ ("start" | "end")] if border => side_parts(end),
        [part] if border => vec![
            format!("{prefix}-{axis}-start-{part}"),
            format!("{prefix}-{axis}-end-{part}"),
        ],
        _ => return None,
    })
}

/// The flow-relative longhands, as the reference browser lists them in a
/// computed style.
pub(crate) const LONGHANDS: &[&str] = &[
    "block-size",
    "border-block-end-color",
    "border-block-end-style",
    "border-block-end-width",
    "border-block-start-color",
    "border-block-start-style",
    "border-block-start-width",
    "border-end-end-radius",
    "border-end-start-radius",
    "border-inline-end-color",
    "border-inline-end-style",
    "border-inline-end-width",
    "border-inline-start-color",
    "border-inline-start-style",
    "border-inline-start-width",
    "border-start-end-radius",
    "border-start-start-radius",
    "inline-size",
    "inset-block-end",
    "inset-block-start",
    "inset-inline-end",
    "inset-inline-start",
    "margin-block-end",
    "margin-block-start",
    "margin-inline-end",
    "margin-inline-start",
    "max-block-size",
    "max-inline-size",
    "min-block-size",
    "min-inline-size",
    "overflow-block",
    "overflow-inline",
    "padding-block-end",
    "padding-block-start",
    "padding-inline-end",
    "padding-inline-start",
    "scroll-margin-block-end",
    "scroll-margin-block-start",
    "scroll-margin-inline-end",
    "scroll-margin-inline-start",
    "scroll-padding-block-end",
    "scroll-padding-block-start",
    "scroll-padding-inline-end",
    "scroll-padding-inline-start",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str, direction: Direction) -> String {
        match map(name, direction) {
            Some(Mapped::One(physical)) => physical,
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn each_longhand_maps_to_its_side_by_the_direction() {
        let cases = [
            ("margin-inline-start", "margin-left", "margin-right"),
            ("margin-inline-end", "margin-right", "margin-left"),
            ("padding-block-start", "padding-top", "padding-top"),
            ("inset-inline-start", "left", "right"),
            ("inset-block-end", "bottom", "bottom"),
            (
                "border-inline-start-width",
                "border-left-width",
                "border-right-width",
            ),
            (
                "border-block-end-color",
                "border-bottom-color",
                "border-bottom-color",
            ),
            (
                "border-start-end-radius",
                "border-top-right-radius",
                "border-top-left-radius",
            ),
            (
                "border-end-start-radius",
                "border-bottom-left-radius",
                "border-bottom-right-radius",
            ),
            (
                "scroll-padding-inline-end",
                "scroll-padding-right",
                "scroll-padding-left",
            ),
            ("inline-size", "width", "width"),
            ("max-block-size", "max-height", "max-height"),
            ("overflow-inline", "overflow-x", "overflow-x"),
        ];
        for (name, ltr, rtl) in cases {
            assert_eq!(one(name, Direction::Ltr), ltr, "{name}");
            assert_eq!(one(name, Direction::Rtl), rtl, "{name}");
            assert_eq!(
                physical_longhand(name, Direction::Rtl).as_deref(),
                Some(rtl)
            );
        }
        // A border side is a shorthand, not a longhand.
        assert_eq!(one("border-inline-start", Direction::Rtl), "border-right");
        assert_eq!(
            physical_longhand("border-inline-start", Direction::Rtl),
            None
        );
        for name in [
            "margin",
            "margin-top",
            "border-left",
            "width",
            "margin-inline-middle",
        ] {
            assert_eq!(map(name, Direction::Ltr), None, "{name}");
        }
    }

    #[test]
    fn the_shorthands_set_both_ends_or_both_sides() {
        assert_eq!(
            map("padding-inline", Direction::Rtl),
            Some(Mapped::Pair([
                "padding-right".to_owned(),
                "padding-left".to_owned()
            ]))
        );
        assert_eq!(
            map("border-block-style", Direction::Ltr),
            Some(Mapped::Pair([
                "border-top-style".to_owned(),
                "border-bottom-style".to_owned()
            ]))
        );
        assert_eq!(
            map("border-inline", Direction::Ltr),
            Some(Mapped::Both([
                "border-left".to_owned(),
                "border-right".to_owned()
            ]))
        );
        assert_eq!(
            longhands_of("margin-block").unwrap(),
            ["margin-block-start", "margin-block-end"]
        );
        assert_eq!(longhands_of("border-inline-end").unwrap().len(), 3);
        assert_eq!(longhands_of("border-block").unwrap().len(), 6);
        assert_eq!(longhands_of("margin-inline-start"), None);
        // Every longhand listed is one.
        for name in LONGHANDS {
            assert!(physical_longhand(name, Direction::Ltr).is_some(), "{name}");
        }
    }
}
