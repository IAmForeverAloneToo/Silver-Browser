//! The track sizing algorithm (css-grid-2 §12.3 to §12.8, M18), written
//! once for an axis: tracks with their minimum and maximum sizing
//! functions, the gutters between them, and the items' spans and size
//! contributions in the axis. The grid's columns and its rows are sized
//! by the same code, and the tests size them without a box tree.
//!
//! Sizes are `f64` while the algorithm runs, so that sums of large
//! values and of many tracks stay finite and exact enough; a growth
//! limit may be infinite until §12.5 ends. Gutters are fixed tracks
//! between the tracks (§11.1): an item crossing one has its size taken
//! out of its contribution, and no space is ever distributed to one.

use std::collections::BTreeMap;
use std::ops::Range;

/// A minimum or maximum track sizing function (§12.2), its length
/// resolved: a percentage of a size not known is `auto` (§7.2.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Sizing {
    Fixed(f32),
    MinContent,
    MaxContent,
    Auto,
    /// A flex factor, in `fr`; only ever a maximum.
    Flex(f32),
}

impl Sizing {
    /// Whether this is an intrinsic sizing function: `min-content`,
    /// `max-content` or `auto` (`fit-content()` is a `max-content`
    /// maximum with a limit).
    fn is_intrinsic(self) -> bool {
        matches!(self, Sizing::MinContent | Sizing::MaxContent | Sizing::Auto)
    }
}

/// One track while it is sized: its sizing functions and, once sized,
/// its base size.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Track {
    pub(crate) min: Sizing,
    /// `auto` and `fit-content()` count as `max-content` here except where
    /// §12 says otherwise.
    pub(crate) max: Sizing,
    /// The argument of a `fit-content()` track, which limits it.
    pub(crate) fit_content: Option<f32>,
    base: f64,
    limit: f64,
    /// §12.5's "infinitely growable", for one step of one span's items.
    infinitely_growable: bool,
}

impl Track {
    /// A track sized by `min` and `max`, limited by `fit_content` when
    /// it is a `fit-content()` track.
    pub(crate) fn new(min: Sizing, max: Sizing, fit_content: Option<f32>) -> Self {
        Track {
            min,
            max,
            fit_content,
            base: 0.0,
            limit: 0.0,
            infinitely_growable: false,
        }
    }

    /// The track's size once [`size_tracks`] is done: its base size.
    pub(crate) fn size(&self) -> f32 {
        self.base as f32
    }

    fn is_flexible(&self) -> bool {
        matches!(self.max, Sizing::Flex(_))
    }

    fn flex(&self) -> f64 {
        match self.max {
            Sizing::Flex(f) => f64::from(f),
            _ => 0.0,
        }
    }

    /// Whether the maximum is `max-content`, as `auto` and `fit-content()`
    /// count.
    fn max_content_max(&self) -> bool {
        matches!(self.max, Sizing::MaxContent | Sizing::Auto)
    }

    /// The growth limit, an infinite one as the base size, as §12.5.1
    /// reads it when it finds the space to distribute.
    fn finite_limit(&self) -> f64 {
        if self.limit.is_finite() {
            self.limit
        } else {
            self.base
        }
    }
}

/// An item's span of tracks in the axis and its outer size contributions
/// there (§12.5): its minimum contribution, and its min-content and
/// max-content contributions.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Contribution {
    pub(crate) span: Range<usize>,
    pub(crate) minimum: f32,
    pub(crate) min_content: f32,
    pub(crate) max_content: f32,
}

/// The available grid space in the axis (§12.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Available {
    /// The container's content box is this size.
    Definite(f32),
    /// The container is sized under a min-content constraint.
    MinContent,
    /// The container is sized under a max-content constraint, as an
    /// `auto` block size is (§5.2).
    MaxContent,
}

/// What the tracks are sized in: the available space, the container's
/// definite minimum and maximum content-box size in the axis, the
/// gutter after each track but the last, and whether the axis's
/// `justify-content` or `align-content` stretches `auto` tracks
/// (`normal` or `stretch`, §12.8).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Space {
    pub(crate) available: Available,
    pub(crate) min: Option<f32>,
    pub(crate) max: Option<f32>,
    pub(crate) gutters: Vec<f32>,
    pub(crate) stretch: bool,
}

impl Space {
    /// The gutters between the tracks of `span`.
    fn gutters_in(&self, span: &Range<usize>) -> f64 {
        if span.len() < 2 {
            return 0.0;
        }
        self.gutters[span.start..span.end - 1]
            .iter()
            .map(|&g| f64::from(g))
            .sum()
    }

    fn all_gutters(&self) -> f64 {
        self.gutters.iter().map(|&g| f64::from(g)).sum()
    }
}

/// Sizes `tracks` for `items` in `space`: §12.4 to §12.8. Each track's
/// [`Track::size`] is then its used size.
pub(crate) fn size_tracks(tracks: &mut [Track], items: &[Contribution], space: &Space) {
    debug_assert_eq!(space.gutters.len(), tracks.len().saturating_sub(1));
    initialize(tracks);
    resolve_intrinsic(tracks, items, space);
    maximize(tracks, space);
    expand_flexible(tracks, items, space);
    stretch_auto(tracks, space);
}

/// §12.4: a fixed minimum is the base size and a fixed maximum the
/// growth limit; otherwise zero and infinity; the limit never below the
/// base.
fn initialize(tracks: &mut [Track]) {
    for t in tracks {
        t.base = match t.min {
            Sizing::Fixed(v) => f64::from(v),
            _ => 0.0,
        };
        t.limit = match t.max {
            Sizing::Fixed(v) => f64::from(v),
            _ => f64::INFINITY,
        };
        if t.limit < t.base {
            t.limit = t.base;
        }
    }
}

/// Which size a distribution grows (§12.5.1's "affected size").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Affected {
    Base,
    Limit,
}

/// Which of an item's contributions a distribution accommodates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Minimum,
    LimitedMinContent,
    MinContent,
    LimitedMaxContent,
    MaxContent,
}

/// §12.5: the intrinsic track sizes, from the items that span one track,
/// then those spanning more a span at a time, then those crossing a
/// flexible track all together; an infinite growth limit left over
/// becomes the base size.
fn resolve_intrinsic(tracks: &mut [Track], items: &[Contribution], space: &Space) {
    let crosses_flexible = |c: &Contribution| c.span.clone().any(|t| tracks[t].is_flexible());
    let mut by_span: BTreeMap<usize, Vec<&Contribution>> = BTreeMap::new();
    let mut flexible = Vec::new();
    for item in items.iter().filter(|c| !c.span.is_empty()) {
        if crosses_flexible(item) {
            flexible.push(item);
        } else {
            by_span.entry(item.span.len()).or_default().push(item);
        }
    }
    for group in by_span.values() {
        accommodate(tracks, group, space, false);
    }
    if !flexible.is_empty() {
        accommodate(tracks, &flexible, space, true);
    }
    for t in tracks {
        if t.limit.is_infinite() {
            t.limit = t.base;
        }
    }
}

/// One group's steps of §12.5 (the span-1 rules being these steps for a
/// span of one): the base sizes of tracks with intrinsic minimums, then
/// content-based, then max-content minimums; then the growth limits of
/// tracks with intrinsic, then max-content maximums. With `flexible`,
/// only the flexible tracks grow, in proportion to their factors.
///
/// Under a min-content or max-content constraint, an item's limited
/// min-content contribution stands in for its minimum one (step 3.1),
/// but for the items crossing flexible tracks under a min-content
/// constraint: the reference browser and the web-platform-tests
/// (grid-flex-spanning-items-001) keep their minimum contribution there,
/// which the draft's "repeat the previous step" would not.
fn accommodate(tracks: &mut [Track], group: &[&Contribution], space: &Space, flexible: bool) {
    let under_max = space.available == Available::MaxContent;
    for t in tracks.iter_mut() {
        t.infinitely_growable = false;
    }
    let only = |t: &Track, ok: bool| ok && (!flexible || t.is_flexible());
    let minimum = match space.available {
        Available::MinContent if !flexible => Kind::LimitedMinContent,
        Available::MaxContent => Kind::LimitedMinContent,
        _ => Kind::Minimum,
    };
    distribute(
        tracks,
        group,
        space,
        Affected::Base,
        minimum,
        flexible,
        &|t| only(t, t.min.is_intrinsic()),
    );
    distribute(
        tracks,
        group,
        space,
        Affected::Base,
        Kind::MinContent,
        flexible,
        &|t| only(t, matches!(t.min, Sizing::MinContent | Sizing::MaxContent)),
    );
    if under_max {
        distribute(
            tracks,
            group,
            space,
            Affected::Base,
            Kind::LimitedMaxContent,
            flexible,
            &|t| only(t, matches!(t.min, Sizing::Auto | Sizing::MaxContent)),
        );
    }
    distribute(
        tracks,
        group,
        space,
        Affected::Base,
        Kind::MaxContent,
        flexible,
        &|t| only(t, t.min == Sizing::MaxContent),
    );
    for t in tracks.iter_mut() {
        if t.limit < t.base {
            t.limit = t.base;
        }
    }
    distribute(
        tracks,
        group,
        space,
        Affected::Limit,
        Kind::MinContent,
        flexible,
        &|t| only(t, t.max.is_intrinsic()),
    );
    distribute(
        tracks,
        group,
        space,
        Affected::Limit,
        Kind::MaxContent,
        flexible,
        &|t| only(t, t.max_content_max()),
    );
}

/// An item's contribution of `kind`. A limited one (§12.5) is limited by
/// the maximum sizing function of the one track it spans when that is
/// fixed or a `fit-content()` argument, or by the sum of the fixed
/// maximums of the tracks it spans and the gutters between them when
/// every one is fixed, and floored by the minimum contribution.
fn contribution(item: &Contribution, kind: Kind, tracks: &[Track], space: &Space) -> f64 {
    let limited = |value: f32| {
        let fixed = |t: &Track| match t.max {
            Sizing::Fixed(v) => Some(f64::from(v)),
            _ if item.span.len() == 1 => t.fit_content.map(f64::from),
            _ => None,
        };
        let limit: Option<f64> = item
            .span
            .clone()
            .map(|t| fixed(&tracks[t]))
            .sum::<Option<f64>>()
            .map(|sum| sum + space.gutters_in(&item.span));
        let value = f64::from(value);
        limit
            .map_or(value, |limit| value.min(limit))
            .max(f64::from(item.minimum))
    };
    match kind {
        Kind::Minimum => f64::from(item.minimum),
        Kind::LimitedMinContent => limited(item.min_content),
        Kind::MinContent => f64::from(item.min_content),
        Kind::LimitedMaxContent => limited(item.max_content),
        Kind::MaxContent => f64::from(item.max_content),
    }
}

/// §12.5.1: grows the `which` size of the tracks `affected` picks so
/// that each item of `group` has room for its `kind` contribution, by
/// the most any item asks of each track, so that the result does not
/// depend on the items' order.
fn distribute(
    tracks: &mut [Track],
    group: &[&Contribution],
    space: &Space,
    which: Affected,
    kind: Kind,
    flexible: bool,
    affected: &dyn Fn(&Track) -> bool,
) {
    let size_of = |t: &Track| match which {
        Affected::Base => t.base,
        Affected::Limit => t.finite_limit(),
    };
    let mut planned = vec![0.0f64; tracks.len()];
    // The affected tracks some item of the group spans, whose infinite
    // growth limits become finite even when nothing is added to them.
    let mut touched = vec![false; tracks.len()];
    for item in group {
        let spanned: Vec<usize> = item.span.clone().collect();
        let mine: Vec<usize> = spanned
            .iter()
            .copied()
            .filter(|&t| affected(&tracks[t]))
            .collect();
        if mine.is_empty() {
            continue;
        }
        for &t in &mine {
            touched[t] = true;
        }
        let taken: f64 = spanned.iter().map(|&t| size_of(&tracks[t])).sum::<f64>()
            + space.gutters_in(&item.span);
        let mut extra = (contribution(item, kind, tracks, space) - taken).max(0.0);
        if extra <= 0.0 {
            continue;
        }
        let mut incurred = vec![0.0f64; tracks.len()];
        // The limit a track grows to "up to limits".
        let limit = |t: &Track| match which {
            Affected::Base => t.limit.min(t.fit_content.map_or(f64::INFINITY, f64::from)),
            Affected::Limit if t.limit.is_finite() && !t.infinitely_growable => t.limit,
            Affected::Limit => t.fit_content.map_or(f64::INFINITY, f64::from),
        };
        let weights = |t: usize| if flexible { tracks[t].flex() } else { 1.0 };
        extra = fill(
            &mine,
            &|t| size_of(&tracks[t]),
            &|t| limit(&tracks[t]),
            &weights,
            flexible,
            &mut incurred,
            extra,
        );
        // Room left in the spanned tracks not affected keeps the affected
        // ones from passing their limits for it. Beside flexible tracks
        // the others count as fixed, with no room.
        if extra > 0.0 && !flexible {
            let others: Vec<usize> = spanned
                .iter()
                .copied()
                .filter(|&t| !affected(&tracks[t]))
                .collect();
            if !others.is_empty() {
                let mut absorbed = vec![0.0f64; tracks.len()];
                extra = fill(
                    &others,
                    &|t| size_of(&tracks[t]),
                    &|t| limit(&tracks[t]),
                    &|_| 1.0,
                    false,
                    &mut absorbed,
                    extra,
                );
            }
        }
        // Beyond the limits: into the tracks whose maximum lets them grow.
        // A `fit-content()` track grows as `max-content` up to its
        // argument, then as a fixed one, which stops it. A base size left
        // with space when none of them can take more, there being none or
        // all at their arguments, grows in every affected track (below).
        if extra > 0.0 {
            let grows = |t: &Track| match (which, kind) {
                (Affected::Base, Kind::MaxContent | Kind::LimitedMaxContent) => t.max_content_max(),
                _ => t.max.is_intrinsic(),
            };
            let beyond: Vec<usize> = mine
                .iter()
                .copied()
                .filter(|&t| grows(&tracks[t]))
                .collect();
            let cap = |t: usize| tracks[t].fit_content.map_or(f64::INFINITY, f64::from);
            extra = fill(
                &beyond,
                &|t| size_of(&tracks[t]),
                &cap,
                &weights,
                flexible,
                &mut incurred,
                extra,
            );
            if extra > 0.0 && which == Affected::Base {
                fill(
                    &mine,
                    &|t| size_of(&tracks[t]),
                    &|_| f64::INFINITY,
                    &weights,
                    flexible,
                    &mut incurred,
                    extra,
                );
            }
        }
        for &t in &mine {
            planned[t] = planned[t].max(incurred[t]);
        }
    }
    // The planned increases, an infinite growth limit becoming the base
    // size plus its increase; one made finite by the step for intrinsic
    // maximums is "infinitely growable" for the next.
    for (t, track) in tracks.iter_mut().enumerate() {
        if !touched[t] {
            continue;
        }
        let increase = planned[t];
        match which {
            Affected::Base => track.base += increase,
            Affected::Limit if track.limit.is_infinite() => {
                track.limit = track.base + increase;
                track.infinitely_growable = kind == Kind::MinContent;
            }
            Affected::Limit => track.limit += increase,
        }
    }
}

/// Shares `space` among `tracks`, each growing from `current` plus what
/// it has `incurred` so far, equally or by `weights`, and freezing at
/// its `limit` while the rest go on; returns what is left. With
/// `flexible` and factors summing to less than one, only that share of
/// the space goes by the factors and the rest equally (§12.5).
fn fill(
    tracks: &[usize],
    current: &dyn Fn(usize) -> f64,
    limit: &dyn Fn(usize) -> f64,
    weights: &dyn Fn(usize) -> f64,
    flexible: bool,
    incurred: &mut [f64],
    space: f64,
) -> f64 {
    if flexible {
        let sum: f64 = tracks.iter().map(|&t| weights(t)).sum();
        if sum < 1.0 {
            let by_factor = space * sum;
            let left = water(tracks, current, limit, weights, incurred, by_factor);
            return water(
                tracks,
                current,
                limit,
                &|_| 1.0,
                incurred,
                space - by_factor + left,
            );
        }
    }
    water(tracks, current, limit, weights, incurred, space)
}

/// [`fill`]'s sharing: each unfrozen track takes its weight's share, the
/// first to reach its limit freezing there, until the space is gone or
/// every track is frozen.
fn water(
    tracks: &[usize],
    current: &dyn Fn(usize) -> f64,
    limit: &dyn Fn(usize) -> f64,
    weights: &dyn Fn(usize) -> f64,
    incurred: &mut [f64],
    mut space: f64,
) -> f64 {
    const EPSILON: f64 = 1e-9;
    let mut frozen: Vec<bool> = tracks.iter().map(|&t| weights(t) <= 0.0).collect();
    while space > EPSILON {
        let open: Vec<(usize, usize)> = tracks
            .iter()
            .copied()
            .enumerate()
            .filter(|&(i, t)| !frozen[i] && current(t) + incurred[t] < limit(t) - EPSILON)
            .collect();
        if open.is_empty() {
            break;
        }
        let total: f64 = open.iter().map(|&(_, t)| weights(t)).sum();
        let share = space / total;
        let (tightest, room) = open
            .iter()
            .map(|&(i, t)| (i, (limit(t) - current(t) - incurred[t]) / weights(t)))
            .fold(
                (usize::MAX, f64::INFINITY),
                |a, b| if b.1 < a.1 { b } else { a },
            );
        let step = share.min(room);
        for &(_, t) in &open {
            incurred[t] += step * weights(t);
        }
        space -= step * total;
        if step >= share {
            space = 0.0;
        } else if tightest != usize::MAX {
            frozen[tightest] = true;
        }
    }
    space.max(0.0)
}

/// The grid's size: its tracks' base sizes and the gutters.
fn grid_size(tracks: &[Track], space: &Space) -> f64 {
    tracks.iter().map(|t| t.base).sum::<f64>() + space.all_gutters()
}

/// The free space (§12.2): the available space less the grid's size,
/// never negative; infinite under a max-content constraint and zero
/// under a min-content one, as §12.6 takes them.
fn free_space(tracks: &[Track], space: &Space, available: Available) -> f64 {
    match available {
        Available::Definite(size) => (f64::from(size) - grid_size(tracks, space)).max(0.0),
        Available::MaxContent => f64::INFINITY,
        Available::MinContent => 0.0,
    }
}

/// §12.6: the free space goes to the base sizes equally, each track
/// frozen at its growth limit; again in the container's maximum size if
/// the grid would pass it.
fn maximize(tracks: &mut [Track], space: &Space) {
    let before: Vec<f64> = tracks.iter().map(|t| t.base).collect();
    let grow = |tracks: &mut [Track], free: f64| {
        if free <= 0.0 {
            return;
        }
        let all: Vec<usize> = (0..tracks.len()).collect();
        let mut incurred = vec![0.0f64; tracks.len()];
        let current: Vec<f64> = tracks.iter().map(|t| t.base).collect();
        let limits: Vec<f64> = tracks.iter().map(|t| t.limit).collect();
        if free.is_infinite() {
            for t in tracks.iter_mut() {
                t.base = t.limit.max(t.base);
            }
            return;
        }
        water(
            &all,
            &|t| current[t],
            &|t| limits[t],
            &|_| 1.0,
            &mut incurred,
            free,
        );
        for (t, track) in tracks.iter_mut().enumerate() {
            track.base += incurred[t];
        }
    };
    grow(tracks, free_space(tracks, space, space.available));
    if let Some(max) = space.max
        && grid_size(tracks, space) > f64::from(max) + 1e-6
    {
        for (t, base) in tracks.iter_mut().zip(before) {
            t.base = base;
        }
        let free = free_space(tracks, space, Available::Definite(max));
        grow(tracks, free);
    }
}

/// §12.7.1: the largest size an `fr` can be with the tracks `of` filling
/// `fill`: what the inflexible tracks and the gutters leave, over the
/// flex factors (at least one), a flexible track whose share would fall
/// below its base size counting as inflexible.
fn fr_size(tracks: &[Track], of: Range<usize>, fill: f64, space: &Space) -> f64 {
    let mut inflexible: Vec<bool> = of.clone().map(|t| !tracks[t].is_flexible()).collect();
    loop {
        let leftover = fill
            - space.gutters_in(&of)
            - of.clone()
                .zip(&inflexible)
                .filter(|&(_, &rigid)| rigid)
                .map(|(t, _)| tracks[t].base)
                .sum::<f64>();
        let factors: f64 = of
            .clone()
            .zip(&inflexible)
            .filter(|&(_, &rigid)| !rigid)
            .map(|(t, _)| tracks[t].flex())
            .sum();
        let hypothetical = leftover / factors.max(1.0);
        let mut changed = false;
        for (i, t) in of.clone().enumerate() {
            if !inflexible[i] && hypothetical * tracks[t].flex() < tracks[t].base {
                inflexible[i] = true;
                changed = true;
            }
        }
        if !changed {
            return hypothetical.max(0.0);
        }
    }
}

/// §12.7: the flexible tracks take the used flex fraction times their
/// factor, if that is more than their base size.
fn expand_flexible(tracks: &mut [Track], items: &[Contribution], space: &Space) {
    if !tracks.iter().any(Track::is_flexible) {
        return;
    }
    let all = 0..tracks.len();
    let fraction = |tracks: &[Track], available: Available| match available {
        Available::MinContent => 0.0,
        Available::Definite(size) => {
            if free_space(tracks, space, available) <= 0.0 {
                0.0
            } else {
                fr_size(tracks, all.clone(), f64::from(size), space)
            }
        }
        Available::MaxContent => {
            let tracks_fraction = tracks
                .iter()
                .filter(|t| t.is_flexible())
                .map(|t| {
                    let f = t.flex();
                    if f > 1.0 { t.base / f } else { t.base }
                })
                .fold(0.0, f64::max);
            items
                .iter()
                .filter(|c| c.span.clone().any(|t| tracks[t].is_flexible()))
                .map(|c| fr_size(tracks, c.span.clone(), f64::from(c.max_content), space))
                .fold(tracks_fraction, f64::max)
        }
    };
    let size_with = |tracks: &[Track], fraction: f64| {
        tracks
            .iter()
            .map(|t| {
                if t.is_flexible() {
                    t.base.max(fraction * t.flex())
                } else {
                    t.base
                }
            })
            .sum::<f64>()
            + space.all_gutters()
    };
    let mut used = fraction(tracks, space.available);
    if let Some(min) = space.min
        && size_with(tracks, used) < f64::from(min) - 1e-6
    {
        used = fr_size(tracks, all.clone(), f64::from(min), space);
    } else if let Some(max) = space.max
        && size_with(tracks, used) > f64::from(max) + 1e-6
    {
        used = fr_size(tracks, all.clone(), f64::from(max), space);
    }
    for t in tracks.iter_mut().filter(|t| t.is_flexible()) {
        t.base = t.base.max(used * t.flex());
    }
}

/// §12.8: with `normal` or `stretch` content distribution, the tracks
/// with an `auto` maximum share the positive free space equally, against
/// the container's definite minimum size when its own is indefinite. A
/// `fit-content()` track's maximum is `max-content`, so it never does.
fn stretch_auto(tracks: &mut [Track], space: &Space) {
    if !space.stretch {
        return;
    }
    let free = match space.available {
        Available::Definite(size) => f64::from(size) - grid_size(tracks, space),
        _ => match space.min {
            Some(min) => f64::from(min) - grid_size(tracks, space),
            None => return,
        },
    };
    let auto: Vec<usize> = (0..tracks.len())
        .filter(|&t| tracks[t].max == Sizing::Auto)
        .collect();
    if free <= 0.0 || auto.is_empty() {
        return;
    }
    let share = free / auto.len() as f64;
    for t in auto {
        tracks[t].base += share;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Sizing::{Auto, Fixed, Flex, MaxContent, MinContent};

    fn track(min: Sizing, max: Sizing) -> Track {
        Track::new(min, max, None)
    }

    /// A `1fr`-style track: an `auto` minimum and a flexible maximum.
    fn fr(factor: f32) -> Track {
        track(Auto, Flex(factor))
    }

    fn item(span: Range<usize>, minimum: f32, min_content: f32, max_content: f32) -> Contribution {
        Contribution {
            span,
            minimum,
            min_content,
            max_content,
        }
    }

    fn space(available: Available, tracks: usize) -> Space {
        Space {
            available,
            min: None,
            max: None,
            gutters: vec![0.0; tracks.saturating_sub(1)],
            stretch: false,
        }
    }

    fn sized(mut tracks: Vec<Track>, items: &[Contribution], space: &Space) -> Vec<f32> {
        size_tracks(&mut tracks, items, space);
        tracks.iter().map(Track::size).collect()
    }

    #[test]
    fn fixed_and_flexible_tracks_share_the_space() {
        // §7.2.1's example, `100px 1fr max-content minmax(min-content,
        // 1fr)`: the flexible tracks halve what the others leave.
        let tracks = vec![
            track(Fixed(100.0), Fixed(100.0)),
            fr(1.0),
            track(MaxContent, MaxContent),
            track(MinContent, Flex(1.0)),
        ];
        let items = [item(2..3, 30.0, 30.0, 50.0), item(3..4, 20.0, 20.0, 40.0)];
        assert_eq!(
            sized(
                tracks.clone(),
                &items,
                &space(Available::Definite(500.0), 4)
            ),
            vec![100.0, 175.0, 50.0, 175.0]
        );
        // When the rest are larger than the container, the flexible
        // tracks are nothing but their minimums.
        assert_eq!(
            sized(tracks, &items, &space(Available::Definite(150.0), 4)),
            vec![100.0, 0.0, 50.0, 20.0]
        );
    }

    #[test]
    fn an_infinitely_growable_limit_lets_the_spanned_track_take_the_rest() {
        // §12.5's note: two `minmax(min-content, max-content)` tracks, an
        // item of 10 in the first and one of 30 to 100 across both. The
        // first track stays exactly as large as its item.
        let tracks = vec![track(MinContent, MaxContent); 2];
        let items = [item(0..1, 10.0, 10.0, 10.0), item(0..2, 30.0, 30.0, 100.0)];
        assert_eq!(
            sized(tracks, &items, &space(Available::Definite(100.0), 2)),
            vec![10.0, 90.0]
        );
    }

    #[test]
    fn auto_tracks_grow_to_their_content_and_then_stretch() {
        let tracks = vec![track(Auto, Auto); 2];
        let items = [item(0..1, 10.0, 10.0, 50.0), item(1..2, 10.0, 10.0, 50.0)];
        let mut s = space(Available::Definite(300.0), 2);
        assert_eq!(sized(tracks.clone(), &items, &s), vec![50.0, 50.0]);
        s.stretch = true;
        assert_eq!(sized(tracks, &items, &s), vec![150.0, 150.0]);
    }

    #[test]
    fn a_flexible_track_whose_share_is_too_small_keeps_its_base_size() {
        // §12.7.1: the first `1fr` needs 250 of 300, more than half, so
        // it is taken as inflexible and the other has what is left.
        let items = [item(0..1, 250.0, 250.0, 250.0)];
        assert_eq!(
            sized(
                vec![fr(1.0), fr(1.0)],
                &items,
                &space(Available::Definite(300.0), 2)
            ),
            vec![250.0, 50.0]
        );
    }

    #[test]
    fn flexible_tracks_in_an_indefinite_space_keep_their_proportions() {
        // The reference browser's float with columns `1fr 2fr` and two
        // items 30 pixels wide: 30 and 60.
        let items = [item(0..1, 30.0, 30.0, 30.0), item(1..2, 30.0, 30.0, 30.0)];
        assert_eq!(
            sized(
                vec![fr(1.0), fr(2.0)],
                &items,
                &space(Available::MaxContent, 2)
            ),
            vec![30.0, 60.0]
        );
        // Under a min-content constraint, a flexible track is its base.
        assert_eq!(
            sized(
                vec![fr(1.0), fr(2.0)],
                &items,
                &space(Available::MinContent, 2)
            ),
            vec![30.0, 30.0]
        );
    }

    #[test]
    fn flex_factors_below_one_leave_part_of_the_space() {
        // §7.2.4: two `0.25fr` tracks take a quarter each.
        assert_eq!(
            sized(
                vec![fr(0.25), fr(0.25)],
                &[],
                &space(Available::Definite(400.0), 2)
            ),
            vec![100.0, 100.0]
        );
    }

    #[test]
    fn the_constraint_decides_an_auto_tracks_size() {
        let items = [item(0..1, 25.0, 25.0, 54.0)];
        let one = || vec![track(Auto, Auto)];
        assert_eq!(
            sized(one(), &items, &space(Available::MinContent, 1)),
            vec![25.0]
        );
        assert_eq!(
            sized(one(), &items, &space(Available::MaxContent, 1)),
            vec![54.0]
        );
        assert_eq!(
            sized(one(), &items, &space(Available::Definite(40.0), 1)),
            vec![40.0]
        );
    }

    #[test]
    fn fit_content_is_max_content_limited_by_its_argument() {
        // The reference browser's `fit-content(40px)` column over content
        // of 25 to 54: 40.
        let fit = || vec![Track::new(Auto, MaxContent, Some(40.0))];
        let wide = [item(0..1, 25.0, 25.0, 54.0)];
        assert_eq!(
            sized(fit(), &wide, &space(Available::Definite(1000.0), 1)),
            vec![40.0]
        );
        assert_eq!(
            sized(fit(), &wide, &space(Available::MaxContent, 1)),
            vec![40.0]
        );
        let narrow = [item(0..1, 25.0, 25.0, 30.0)];
        assert_eq!(
            sized(fit(), &narrow, &space(Available::Definite(1000.0), 1)),
            vec![30.0]
        );
        // Content wider than the argument at its narrowest still fits.
        let long = [item(0..1, 60.0, 60.0, 90.0)];
        assert_eq!(
            sized(fit(), &long, &space(Available::Definite(1000.0), 1)),
            vec![60.0]
        );
        // It does not stretch as an `auto` track does.
        let mut s = space(Available::Definite(1000.0), 1);
        s.stretch = true;
        assert_eq!(sized(fit(), &narrow, &s), vec![30.0]);
    }

    #[test]
    fn an_auto_minimum_under_a_fixed_maximum_is_limited_by_it() {
        // The reference browser's `minmax(auto, 30px)` over content of 25
        // to 54: 30 at its widest, the minimum contribution at its
        // narrowest.
        let items = [item(0..1, 25.0, 25.0, 54.0)];
        let one = || vec![track(Auto, Fixed(30.0))];
        assert_eq!(
            sized(one(), &items, &space(Available::MaxContent, 1)),
            vec![30.0]
        );
        assert_eq!(
            sized(one(), &items, &space(Available::MinContent, 1)),
            vec![25.0]
        );
        assert_eq!(
            sized(one(), &items, &space(Available::Definite(1000.0), 1)),
            vec![30.0]
        );
    }

    #[test]
    fn a_spanning_item_grows_the_tracks_its_neighbours_left_room_in() {
        let items = [
            item(0..1, 10.0, 10.0, 10.0),
            item(0..2, 100.0, 100.0, 100.0),
        ];
        assert_eq!(
            sized(
                vec![track(Auto, Auto); 2],
                &items,
                &space(Available::MaxContent, 2)
            ),
            vec![10.0, 90.0]
        );
        // Larger spans come after smaller ones, whatever the order given.
        let reversed = [
            item(0..2, 100.0, 100.0, 100.0),
            item(0..1, 10.0, 10.0, 10.0),
        ];
        assert_eq!(
            sized(
                vec![track(Auto, Auto); 2],
                &reversed,
                &space(Available::MaxContent, 2)
            ),
            vec![10.0, 90.0]
        );
    }

    #[test]
    fn gutters_are_fixed_tracks_an_item_crosses() {
        let items = [item(0..2, 100.0, 100.0, 100.0)];
        let mut s = space(Available::MaxContent, 2);
        s.gutters = vec![20.0];
        assert_eq!(
            sized(vec![track(Auto, Auto); 2], &items, &s),
            vec![40.0, 40.0]
        );
        // And no space is given to one.
        let mut s = space(Available::Definite(220.0), 2);
        s.gutters = vec![20.0];
        assert_eq!(sized(vec![fr(1.0), fr(1.0)], &[], &s), vec![100.0, 100.0]);
    }

    #[test]
    fn maximizing_stops_at_the_containers_maximum() {
        // §12.6: grown to its growth limit the grid would pass the
        // maximum size, so it grows only to that.
        let mut s = space(Available::MaxContent, 1);
        s.max = Some(60.0);
        let ranged = || vec![track(Fixed(10.0), Fixed(100.0))];
        assert_eq!(sized(ranged(), &[], &s), vec![60.0]);
        s.max = None;
        assert_eq!(sized(ranged(), &[], &s), vec![100.0]);
        // An `auto` minimum takes the max-content contribution under a
        // max-content constraint before maximizing: the maximum is the
        // container's to apply, not the tracks'.
        let items = [item(0..1, 10.0, 10.0, 100.0)];
        s.max = Some(60.0);
        assert_eq!(sized(vec![track(Auto, Auto)], &items, &s), vec![100.0]);
    }

    #[test]
    fn a_flexible_grid_in_an_indefinite_space_fills_the_containers_minimum() {
        // §12.7: the fraction found from the content would leave the grid
        // smaller than the container's minimum size.
        let items = [item(0..1, 10.0, 10.0, 10.0)];
        let mut s = space(Available::MaxContent, 1);
        s.min = Some(100.0);
        assert_eq!(sized(vec![fr(1.0)], &items, &s), vec![100.0]);
        // §12.8: with only a definite minimum, auto tracks stretch to it.
        s.stretch = true;
        assert_eq!(sized(vec![track(Auto, Auto)], &items, &s), vec![100.0]);
    }

    #[test]
    fn a_base_size_past_a_fixed_maximum_lifts_the_growth_limit() {
        // §12.5 step 3: a growth limit below the base size rises to it, or
        // the item spanning both tracks would find the first smaller than
        // it is and give the second too much. The reference browser:
        // `minmax(auto, 30px) auto`, an item 60 at least in the first and
        // one of 200 across both: 60 and 140.
        let tracks = vec![track(Auto, Fixed(30.0)), track(Auto, Auto)];
        let items = [item(0..1, 60.0, 60.0, 60.0), item(0..2, 0.0, 0.0, 200.0)];
        assert_eq!(
            sized(tracks, &items, &space(Available::Definite(1000.0), 2)),
            vec![60.0, 140.0]
        );
    }

    #[test]
    fn items_crossing_flexible_tracks_grow_only_those_by_their_factors() {
        // §12.5 step 4, as the reference browser and the web-platform-tests
        // (grid-flex-track-intrinsic-sizes-003) have it: `1fr 3fr` 50
        // wide, an item of 100 across both.
        let items = [item(0..2, 100.0, 100.0, 100.0)];
        assert_eq!(
            sized(
                vec![fr(1.0), fr(3.0)],
                &items,
                &space(Available::Definite(50.0), 2)
            ),
            vec![25.0, 75.0]
        );
        // The intrinsic track beside a flexible one counts as fixed: the
        // test's `0.5fr auto` with an item of 100, the flexible track
        // taking its share by factor and the rest.
        assert_eq!(
            sized(
                vec![track(Auto, Auto), fr(0.5)],
                &items,
                &space(Available::Definite(60.0), 2)
            ),
            vec![0.0, 100.0]
        );
        // Factors summing to zero share equally: the test's `0fr 0fr`
        // and `0fr 0fr 1fr`.
        assert_eq!(
            sized(
                vec![fr(0.0), fr(0.0)],
                &items,
                &space(Available::Definite(60.0), 2)
            ),
            vec![50.0, 50.0]
        );
        assert_eq!(
            sized(
                vec![fr(0.0), fr(0.0), fr(1.0)],
                &items,
                &space(Available::Definite(60.0), 3)
            ),
            vec![50.0, 50.0, 0.0]
        );
        // Under a min-content constraint such an item gives its minimum
        // contribution, here none: `1fr 30px` around an item 300 wide is
        // 30 at its narrowest (grid-flex-spanning-items-001).
        let wide = [item(0..2, 0.0, 300.0, 300.0)];
        let fixed = || track(Fixed(30.0), Fixed(30.0));
        assert_eq!(
            sized(
                vec![fr(1.0), fixed()],
                &wide,
                &space(Available::MinContent, 2)
            ),
            vec![0.0, 30.0]
        );
        assert_eq!(
            sized(
                vec![fr(1.0), fixed()],
                &wide,
                &space(Available::MaxContent, 2)
            ),
            vec![270.0, 30.0]
        );
        // Under a max-content constraint it gives its limited min-content
        // contribution first, so that a small item's minimum in one of the
        // tracks does not unbalance them: the reference browser's
        // `1fr 2fr auto`, an item of 300 across the first two and one of
        // 9 in the first, is 100, 200 and 9.
        let mixed = [
            item(0..2, 0.0, 300.0, 300.0),
            item(0..1, 9.0, 9.0, 9.0),
            item(2..3, 9.0, 9.0, 9.0),
        ];
        assert_eq!(
            sized(
                vec![fr(1.0), fr(2.0), track(Auto, Auto)],
                &mixed,
                &space(Available::MaxContent, 3)
            ),
            vec![100.0, 200.0, 9.0]
        );
        // Factors summing to less than one: that share of the space by
        // factor, the rest equally (§12.5 step 4). The reference browser
        // gives all of it by factor, 40 and 60.
        assert_eq!(
            sized(
                vec![fr(0.2), fr(0.3)],
                &items,
                &space(Available::Definite(50.0), 2)
            ),
            vec![45.0, 55.0]
        );
    }

    #[test]
    fn under_a_constraint_an_auto_minimum_takes_the_limited_min_content_contribution() {
        // §12.5 step 2: an item whose minimum contribution is below its
        // min-content one (`min-width: 0`) still gives a container sized
        // under a min-content constraint its min-content contribution. The
        // reference browser gives the minimum contribution, zero.
        let items = [item(0..1, 0.0, 25.0, 54.0)];
        assert_eq!(
            sized(
                vec![track(Auto, Auto)],
                &items,
                &space(Available::MinContent, 1)
            ),
            vec![25.0]
        );
        // Limited by a fixed maximum, with the gutters an item spans when
        // all the tracks it spans have one, and floored by its minimum.
        let two = vec![track(Auto, Fixed(30.0)); 2];
        let mut s = space(Available::MinContent, 2);
        s.gutters = vec![10.0];
        assert_eq!(
            sized(two, &[item(0..2, 0.0, 100.0, 100.0)], &s),
            vec![30.0, 30.0]
        );
        assert_eq!(
            sized(
                vec![track(Auto, Fixed(30.0))],
                &[item(0..1, 50.0, 100.0, 100.0)],
                &space(Available::MinContent, 1)
            ),
            vec![50.0]
        );
    }

    #[test]
    fn a_spanning_items_min_content_grows_both_kinds_of_content_minimum() {
        // §12.5 step 3.2: content-based minimums are `min-content` and
        // `max-content` ones alike.
        let tracks = vec![track(MinContent, Auto), track(MaxContent, Auto)];
        assert_eq!(
            sized(
                tracks,
                &[item(0..2, 0.0, 100.0, 100.0)],
                &space(Available::Definite(1000.0), 2)
            ),
            vec![50.0, 50.0]
        );
        // `minmax(0, min-content)`: the growth limit is the min-content
        // contribution, and the free space grows the track to it (the
        // reference browser, and grid-intrinsic-track-sizes-001).
        assert_eq!(
            sized(
                vec![track(Fixed(0.0), MinContent)],
                &[item(0..1, 0.0, 30.0, 60.0)],
                &space(Available::Definite(1000.0), 1)
            ),
            vec![30.0]
        );
    }

    #[test]
    fn beyond_their_limits_tracks_grow_by_their_maximums() {
        // §12.5.1: a `fit-content()` track's base size grows only to its
        // argument while another affected track can take the rest.
        let fit = |limit| Track::new(Auto, MaxContent, Some(limit));
        let wide = [item(0..2, 100.0, 100.0, 100.0)];
        let definite = space(Available::Definite(1000.0), 2);
        assert_eq!(
            sized(vec![fit(40.0), track(Auto, Auto)], &wide, &definite),
            vec![40.0, 60.0]
        );
        // At its argument it counts as fixed, so with nothing else to grow
        // every affected track shares what is left. The reference browser
        // gives it all to the `fit-content()` track, 80 and 20.
        assert_eq!(
            sized(vec![fit(20.0), track(Auto, Fixed(20.0))], &wide, &definite),
            vec![50.0, 50.0]
        );
        // Space a track not affected has room for goes there before the
        // affected ones pass their limits. The reference browser does not
        // take this step: 100 and 140.
        let tracks = vec![track(Fixed(10.0), Fixed(100.0)), track(Auto, Fixed(20.0))];
        assert_eq!(
            sized(tracks, &[item(0..2, 150.0, 150.0, 150.0)], &definite),
            vec![100.0, 50.0]
        );
        // Max-content contributions beyond the limits go to `max-content`
        // maximums, else to every affected track: the reference browser's
        // `minmax(max-content, min-content) minmax(max-content, 10px)`.
        let tracks = vec![
            track(MaxContent, MinContent),
            track(MaxContent, Fixed(10.0)),
        ];
        let items = [item(0..1, 10.0, 10.0, 10.0), item(0..2, 0.0, 50.0, 100.0)];
        assert_eq!(sized(tracks, &items, &definite), vec![65.0, 35.0]);
    }

    #[test]
    fn the_flex_fraction_in_an_indefinite_space_reads_small_factors_as_one() {
        // §12.7.1: a flexible track's base size over its factor, the
        // factor taken as one when it is less: the reference browser's
        // `0.5fr 1fr` around an item 30 wide is 30 and 30.
        let items = [item(0..1, 30.0, 30.0, 30.0)];
        let mut s = space(Available::MaxContent, 2);
        assert_eq!(sized(vec![fr(0.5), fr(1.0)], &items, &s), vec![30.0, 30.0]);
        // §12.7: a grid that would pass the container's maximum size
        // finds the fraction again in that size.
        s.max = Some(50.0);
        assert_eq!(sized(vec![fr(1.0), fr(1.0)], &items, &s), vec![30.0, 20.0]);
    }

    #[test]
    fn huge_and_many_values_stay_finite() {
        let items: Vec<Contribution> = (0..500)
            .map(|i| item(i % 200..(i % 200) + 1 + i % 3, 1e7, 3e7, 3.3e7))
            .collect();
        let mut tracks = vec![fr(1e30); 100];
        tracks.extend(vec![track(Auto, Auto); 102]);
        let mut s = space(Available::Definite(3.3e7), 202);
        s.gutters = vec![1e6; 201];
        s.stretch = true;
        let sizes = sized(tracks, &items, &s);
        assert!(
            sizes.iter().all(|v| v.is_finite() && *v >= 0.0),
            "{sizes:?}"
        );
    }
}
