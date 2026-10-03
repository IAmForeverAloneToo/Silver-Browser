//! Box alignment (css-align-3), the arithmetic flex (M17) and grid (M18)
//! share: an alignment value resolved for one axis, where it puts its
//! subjects, how a distribution spaces them, the fallbacks of the values
//! that cannot apply, `safe`, and where an absolutely positioned child's
//! static position goes. Each container maps its own keywords to an
//! axis: flex to its flex-relative main and cross axes, which may run
//! from either physical side, grid to axes that run left to right and
//! top to bottom, as everything in layout does.

use style::computed::{
    AlignPosition, BaselinePosition, ContentAlignment, OverflowAlign, SelfAlignment,
};

use crate::block::StaticAlign;

/// Where alignment puts its subjects in one axis, from the axis's start
/// (a flex container's main-start or cross-start, a grid's left or top):
/// at the start, at the end, centred, or spread with the free space
/// between them (css-align-3 §4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Place {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// An alignment value resolved for one axis: where its subjects go, and
/// whether subjects that overflow go to the axis's physical start
/// instead (`safe`, css-align-3 §4.4). Without `safe` the alignment is
/// unsafe, as the reference browsers have it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Alignment {
    place: Place,
    safe: bool,
}

impl Alignment {
    /// `place`, unsafe.
    pub(crate) fn at(place: Place) -> Self {
        Alignment { place, safe: false }
    }

    /// A positional keyword in an axis that runs from its physical start
    /// when `forward`, and is `horizontal` or not. `start` and `end` are
    /// the container's own, which a flex container's `*-reverse` and
    /// `wrap-reverse` do not flip, and so are `self-start` and `self-end`
    /// with one writing mode; `flex-start` and `flex-end` are the axis's
    /// own start and end, which outside a flex container are `start` and
    /// `end`; `left` and `right` are physical in a horizontal axis and
    /// `start` in a vertical one (css-align-3 §4.1).
    pub(crate) fn position(
        position: AlignPosition,
        overflow: OverflowAlign,
        forward: bool,
        horizontal: bool,
    ) -> Self {
        let (start, end) = if forward {
            (Place::Start, Place::End)
        } else {
            (Place::End, Place::Start)
        };
        let place = match position {
            AlignPosition::FlexStart => Place::Start,
            AlignPosition::FlexEnd => Place::End,
            AlignPosition::Center => Place::Center,
            AlignPosition::Start | AlignPosition::SelfStart | AlignPosition::Left => start,
            AlignPosition::End | AlignPosition::SelfEnd => end,
            AlignPosition::Right if horizontal => end,
            AlignPosition::Right => start,
        };
        Alignment {
            place,
            safe: overflow == OverflowAlign::Safe,
        }
    }

    /// The offset of the first of `count` subjects from the axis's start,
    /// and the space added after each, given the `free` space they leave
    /// in an axis that runs from its physical start when `forward`. A
    /// distribution that cannot distribute takes its fallback, as the
    /// reference browsers have it: `start` for `space-between`, unsafe
    /// where css-align-3 would make it safe, and `safe center` for
    /// `space-around` and `space-evenly`.
    pub(crate) fn offsets(self, free: f32, count: usize, forward: bool) -> (f32, f32) {
        let n = count as f32;
        let fallback = match self.place {
            Place::SpaceBetween if count > 1 && free >= 0.0 => return (0.0, free / (n - 1.0)),
            Place::SpaceAround if count > 0 && free >= 0.0 => return (free / n / 2.0, free / n),
            Place::SpaceEvenly if count > 0 && free >= 0.0 => {
                return (free / (n + 1.0), free / (n + 1.0));
            }
            Place::SpaceBetween => Alignment::at(Place::Start),
            Place::SpaceAround | Place::SpaceEvenly => Alignment {
                place: Place::Center,
                safe: true,
            },
            _ => self,
        };
        let place = match fallback.place {
            _ if fallback.safe && free < 0.0 && forward => Place::Start,
            _ if fallback.safe && free < 0.0 => Place::End,
            place => place,
        };
        match place {
            Place::End => (free, 0.0),
            Place::Center => (free / 2.0, 0.0),
            _ => (0.0, 0.0),
        }
    }

    /// Where an absolutely positioned child's margin box goes, physically,
    /// in its static-position rectangle along an axis that runs from its
    /// physical start when `forward`: as the sole subject, a distribution
    /// taking its fallback, `start` for `space-between` and the centre for
    /// the others; and never safe, as in the reference browsers.
    pub(crate) fn static_align(self, forward: bool) -> StaticAlign {
        let at_start = match self.place {
            Place::Start | Place::SpaceBetween => true,
            Place::End => false,
            Place::Center | Place::SpaceAround | Place::SpaceEvenly => return StaticAlign::Center,
        };
        if at_start == forward {
            StaticAlign::Start
        } else {
            StaticAlign::End
        }
    }
}

/// `justify-content` in an axis that runs from its physical start when
/// `forward` and is `horizontal` or not: `normal` and `stretch` place
/// the subjects at the start, what stretches having grown already (a
/// flex container's items by `flex-grow`, §8.2 of css-flexbox-1; a
/// grid's `auto` tracks, §12.8 of css-grid-2); `baseline` is `start`.
pub(crate) fn justify_content(
    value: ContentAlignment,
    forward: bool,
    horizontal: bool,
) -> Alignment {
    match value {
        ContentAlignment::SpaceBetween => Alignment::at(Place::SpaceBetween),
        ContentAlignment::SpaceAround => Alignment::at(Place::SpaceAround),
        ContentAlignment::SpaceEvenly => Alignment::at(Place::SpaceEvenly),
        ContentAlignment::Position(position, overflow) => {
            Alignment::position(position, overflow, forward, horizontal)
        }
        ContentAlignment::Normal | ContentAlignment::Stretch | ContentAlignment::Baseline(_) => {
            Alignment::at(Place::Start)
        }
    }
}

/// `align-content` in an axis that runs from its physical start when
/// `forward` and is `horizontal` or not: as [`justify_content`], but that
/// `last baseline`, which the reference browsers do not parse, takes its
/// css-align-3 fallback, `safe end`.
pub(crate) fn align_content(value: ContentAlignment, forward: bool, horizontal: bool) -> Alignment {
    match value {
        ContentAlignment::Baseline(BaselinePosition::Last) => {
            Alignment::position(AlignPosition::End, OverflowAlign::Safe, forward, horizontal)
        }
        value => justify_content(value, forward, horizontal),
    }
}

/// A self-alignment value, `auto` already resolved, in an axis that runs
/// from its physical start when `forward` and is `horizontal` or not.
/// `normal` and `stretch` place a subject that does not stretch at the
/// start. A baseline value is the baseline-sharing group's to place;
/// for a subject that has no group, it takes its fallback, `start`, or
/// `end` for `last baseline`, which a flex container's `wrap-reverse`
/// does not flip, as in the reference browsers.
pub(crate) fn self_alignment(value: SelfAlignment, forward: bool, horizontal: bool) -> Alignment {
    match value {
        SelfAlignment::Position(position, overflow) => {
            Alignment::position(position, overflow, forward, horizontal)
        }
        SelfAlignment::Baseline(BaselinePosition::First) => Alignment::position(
            AlignPosition::Start,
            OverflowAlign::Default,
            forward,
            horizontal,
        ),
        SelfAlignment::Baseline(BaselinePosition::Last) => Alignment::position(
            AlignPosition::End,
            OverflowAlign::Default,
            forward,
            horizontal,
        ),
        SelfAlignment::Auto | SelfAlignment::Normal | SelfAlignment::Stretch => {
            Alignment::at(Place::Start)
        }
    }
}
