//! The animations' values in the cascade (M22.2): what a target's
//! animations in effect give each of `compute_style`'s passes, which
//! applies them after the normal declarations and before the important
//! ones (Cascade 5 §6.1), the animations in order, a later one's value
//! for a property winning.
//!
//! A sample reads its keyframes at its transformed progress (Web
//! Animations §5.3.4): for each property, the keyframes that set it,
//! with an implicit one at 0% and at 100% where none sets it there,
//! which takes the underlying value — the element's from the cascade
//! below and the animations before; the two keyframes around the
//! progress, mixed by the earlier one's easing with the before flag, or
//! one alone past an end where more than one sits. A keyframe's value is
//! computed at the sample as the element's own declaration of it would
//! be (§5.3.3): its `em` of the animated font size, its `var()` of the
//! animated custom properties, and a `currentcolor` the animated colour,
//! which the last pass computes before every other property for that.
//! A custom property steps discretely, at half way, but one registered
//! with a syntax, whose keyframes are computed by it and interpolated by
//! their type where they can be (D73). A keyframe's `revert` and
//! `revert-layer` are the value of the user agent's declarations (D73),
//! the animations' origin rolling back past the author's.
//!
//! A keyframe whose composite operation, or whose effect's, is `add` or
//! `accumulate` (Web Animations 1 section 5.4.4; Animations 2's
//! `animation-composition`) has its value composited onto the underlying
//! value before the two around the progress are mixed. A script's
//! keyframes (D75) are read as a rule's are, from the effect that holds
//! them.

use std::sync::Arc;

use css::Token;

use crate::animate::{self, Longhand};
use crate::animation::{Interval, interval};
use crate::computed::{BOTTOM, Color, Composite, ComputedStyle, LEFT, RIGHT, TOP};
use crate::custom;
use crate::easing::Easing;
use crate::keyframes::{Keyframe, Keyframes, ScriptKeyframes};
use crate::rule_set::KeyframesInForce;

use super::{
    CssWide, LengthBases, Resolution, Specified, Target, apply_css_wide, apply_declaration,
    apply_revert, canonical_name, declared_font_size, revert_keyword, substituted,
};

/// A target's animations in effect at the timeline's time, in their
/// order.
pub(super) struct Animated<'a> {
    pub(super) samples: Vec<Sampled<'a>>,
}

/// The keyframes an animation runs: a rule's in force, or a script's.
pub(super) enum Frames<'a> {
    Sheet(KeyframesInForce<'a>),
    Script(Arc<ScriptKeyframes>),
}

impl Frames<'_> {
    /// The keyframes, processed.
    pub(super) fn keyframes(&self) -> &Keyframes {
        match self {
            Frames::Sheet(frames) => frames.keyframes,
            Frames::Script(frames) => frames.processed(),
        }
    }

    /// The declarations of `keyframe`, one of [`Frames::keyframes`]', in
    /// order, each with what the substitutions know it by: a sheet's
    /// place, none for a script's, which it may drop before the cascade
    /// ends.
    fn declarations<'s>(
        &'s self,
        keyframe: &'s Keyframe,
    ) -> Box<dyn Iterator<Item = (&'s css::Declaration, Option<usize>)> + 's> {
        match self {
            Frames::Sheet(frames) => Box::new(
                frames
                    .declarations(keyframe)
                    .map(|d| (d, Some(std::ptr::from_ref(d) as usize))),
            ),
            Frames::Script(frames) => Box::new(frames.declarations(keyframe).map(|d| (d, None))),
        }
    }
}

/// One animation in effect at the timeline's time.
pub(super) struct Sampled<'a> {
    /// The keyframes it runs.
    pub(super) keyframes: Frames<'a>,
    /// Its effect's composite operation, a keyframe's own aside.
    pub(super) composite: Composite,
    /// Its transformed progress, where the keyframes are read.
    pub(super) progress: f64,
    /// Its before flag, which the keyframes' easings take too.
    pub(super) before: bool,
    /// The easing of a keyframe that declares none, an implicit one's
    /// included: the animation's `animation-timing-function`.
    pub(super) easing: Easing,
}

/// Where a property's value comes from among a sample's keyframes: a
/// keyframe by its index, or `None` for an implicit one, which gives the
/// underlying value.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Read {
    /// One keyframe alone.
    At(Option<usize>),
    /// Two mixed, `p` of the way from the first, eased.
    Mix {
        from: Option<usize>,
        to: Option<usize>,
        p: f64,
    },
}

impl Read {
    /// The keyframes it reads.
    fn places(self) -> [Option<usize>; 2] {
        match self {
            Read::At(place) => [place, None],
            Read::Mix { from, to, .. } => [from, to],
        }
    }
}

impl Sampled<'_> {
    /// The keyframes, in order.
    fn keyframes(&self) -> &[Keyframe] {
        &self.keyframes.keyframes().keyframes
    }

    /// The composite operation of keyframe `k`, an implicit one's
    /// `replace`: its value is the underlying one already.
    fn composite_of(&self, k: Option<usize>) -> Composite {
        match k {
            Some(k) => self.keyframes()[k].composite.unwrap_or(self.composite),
            None => Composite::Replace,
        }
    }

    /// Where the property whose keyframes `sets` picks out has its value
    /// at the sample: its keyframes, with an implicit one at either end
    /// that none of them is at, around the progress.
    fn read(&self, sets: impl Fn(&Keyframe) -> bool) -> Read {
        let keyframes = self.keyframes();
        let mut places = Vec::new();
        let mut offsets = Vec::new();
        for (index, keyframe) in keyframes.iter().enumerate() {
            if sets(keyframe) {
                if places.is_empty() && keyframe.offset != 0.0 {
                    places.push(None);
                    offsets.push(0.0);
                }
                places.push(Some(index));
                offsets.push(keyframe.offset);
            }
        }
        if places.is_empty() {
            places.push(None);
            offsets.push(0.0);
        }
        if offsets.last() != Some(&1.0) {
            places.push(None);
            offsets.push(1.0);
        }
        match interval(&offsets, self.progress) {
            Interval::At(at) => Read::At(places[at]),
            Interval::Between { from, to, distance } => {
                let easing = places[from]
                    .and_then(|k| keyframes[k].easing.as_ref())
                    .unwrap_or(&self.easing);
                Read::Mix {
                    from: places[from],
                    to: places[to],
                    p: easing.output(distance, self.before),
                }
            }
        }
    }

    /// The last declaration of `name` in keyframe `k`, with what the
    /// substitutions know it by: the one that wins there, every
    /// declaration of a keyframe being valid where it was parsed.
    fn last_declaration(&self, k: usize, name: &str) -> Option<(&css::Declaration, Option<usize>)> {
        let keyframes = self.keyframes();
        self.keyframes
            .declarations(&keyframes[k])
            .filter(|(d, _)| d.name == name)
            .last()
    }
}

impl<'a> Animated<'a> {
    /// Whether a keyframe the animations read declares a revert keyword,
    /// whose value is the user agent's declarations' (D73).
    pub(super) fn reverts(&self) -> bool {
        self.samples.iter().any(|sample| {
            sample.keyframes().iter().any(|keyframe| {
                sample
                    .keyframes
                    .declarations(keyframe)
                    .any(|(d, _)| revert_keyword(&d.value).is_some())
            })
        })
    }

    /// Whether the animations give only longhands among `only` and no
    /// custom property (M35.2): what nothing else in a style is computed
    /// from, so that their values may be set on the style computed
    /// without them.
    pub(super) fn sets_only(&self, only: &[Longhand]) -> bool {
        self.samples.iter().all(|sample| {
            let keyframes = sample.keyframes.keyframes();
            keyframes.custom.is_empty()
                && keyframes
                    .longhands
                    .iter()
                    .all(|longhand| only.contains(longhand))
        })
    }

    /// The custom property declarations the animations give, in order,
    /// for the custom properties' pass: each the declaration of the
    /// keyframe its discrete step takes, none where that is an implicit
    /// keyframe, whose value is the underlying one. Those of the names
    /// `typed` says are registered with a syntax are left out: they are
    /// [`Animated::typed_custom`]'s.
    pub(super) fn custom_declarations(
        &self,
        typed: &dyn Fn(&str) -> bool,
    ) -> Vec<custom::Declaration<'_>> {
        let mut out = Vec::new();
        for sample in &self.samples {
            for name in &sample.keyframes.keyframes().custom {
                if typed(name) {
                    continue;
                }
                let place = match sample.read(|k| k.sets_custom(name)) {
                    Read::At(place) => place,
                    Read::Mix { from, to, p } => {
                        if p < 0.5 {
                            from
                        } else {
                            to
                        }
                    }
                };
                if let Some(k) = place
                    && let Some((declaration, identity)) = sample.last_declaration(k, name)
                {
                    out.push(custom::Declaration {
                        name: &declaration.name,
                        value: &declaration.value,
                        identity,
                        animated: true,
                        attr: false,
                    });
                }
            }
        }
        out
    }

    /// What the animations give the custom properties of the names
    /// `typed` says are registered with a syntax (D73), in order: each the
    /// keyframes around the progress and how far between them, a keyframe
    /// by the value of its last declaration of the name, `None` for an
    /// implicit one, whose value is the underlying one.
    pub(super) fn typed_custom(&self, typed: &dyn Fn(&str) -> bool) -> Vec<TypedMix<'_>> {
        let mut out = Vec::new();
        for sample in &self.samples {
            for name in sample
                .keyframes
                .keyframes()
                .custom
                .iter()
                .filter(|n| typed(n))
            {
                let value = |place: Option<usize>| {
                    place
                        .and_then(|k| sample.last_declaration(k, name))
                        .map(|(d, _)| &*d.value)
                };
                let (from, to, p) = match sample.read(|k| k.sets_custom(name)) {
                    Read::At(place) => (value(place), value(place), 0.0),
                    Read::Mix { from, to, p } => (value(from), value(to), p),
                };
                out.push(TypedMix { name, from, to, p });
            }
        }
        out
    }

    /// Applies the font size the animations give to `style`, which holds
    /// the one from the normal declarations, for the font size's pass: a
    /// keyframe's own `em` and percentage of the parent's, as the
    /// property's always are; a revert keyword's, `reverted`, the user
    /// agent's declarations' size, or the parent's where none was kept.
    pub(super) fn font_size(
        &self,
        style: &mut ComputedStyle,
        parent: &ComputedStyle,
        resolution: &Resolution<'_>,
        reverted: Option<f32>,
    ) {
        for sample in &self.samples {
            if sample
                .keyframes
                .keyframes()
                .longhands
                .binary_search(&Longhand::FontSize)
                .is_err()
            {
                continue;
            }
            let underlying = style.font_size_px;
            let value = |place: Option<usize>| {
                let value = place
                    .and_then(|k| sample.last_declaration(k, "font-size"))
                    .and_then(|(declaration, identity)| {
                        if revert_keyword(&declaration.value).is_some() {
                            return Some(reverted.unwrap_or(parent.font_size_px));
                        }
                        let specified = substituted(
                            &declaration.value,
                            identity,
                            "font-size",
                            style.custom.as_ref(),
                            resolution,
                        );
                        declared_font_size(specified, false, parent, resolution)
                    })
                    .unwrap_or(underlying);
                match sample.composite_of(place) {
                    Composite::Replace => value,
                    Composite::Add | Composite::Accumulate => {
                        animate::font_size(0.0, value + underlying, 1.0)
                    }
                }
            };
            style.font_size_px = match sample.read(|k| k.sets(Longhand::FontSize)) {
                Read::At(place) => value(place),
                Read::Mix { from, to, p } => animate::font_size(value(from), value(to), p),
            };
        }
    }

    /// Applies the other longhands the animations give to `style`, which
    /// holds the normal declarations' values, for the last pass, a
    /// target's those that apply to it: the colour first, which a
    /// keyframe's `currentcolor` is, then the rest. A border colour an
    /// animation sets counts in `border_color_set` as one declared. A
    /// keyframe's revert keyword takes the value `reverted`, the style of
    /// the user agent's declarations, has.
    pub(super) fn apply(
        &self,
        style: &mut ComputedStyle,
        parent: &ComputedStyle,
        target: Target,
        bases: &LengthBases<'_>,
        border_color_set: &mut [bool; 4],
        reverted: Option<&ComputedStyle>,
    ) {
        for color in [true, false] {
            for sample in &self.samples {
                let longhands: Vec<Longhand> = sample
                    .keyframes
                    .keyframes()
                    .longhands
                    .iter()
                    .copied()
                    .filter(|&longhand| {
                        (longhand == Longhand::Color) == color
                            && longhand != Longhand::FontSize
                            && target.applies(longhand.name())
                    })
                    .collect();
                if longhands.is_empty() {
                    continue;
                }
                let mut underlying = style.clone();
                current_color(&mut underlying, border_color_set, style.color);
                let mut values: Vec<Option<ComputedStyle>> =
                    (0..sample.keyframes().len()).map(|_| None).collect();
                for longhand in longhands {
                    let read = sample.read(|k| k.sets(longhand));
                    for k in read.places().into_iter().flatten() {
                        if values[k].is_none() {
                            values[k] = Some(keyframe_style(
                                sample,
                                k,
                                (&underlying, reverted),
                                parent,
                                bases,
                                (!color).then_some(style.color),
                            ));
                        }
                    }
                    // A keyframe whose operation is not `replace` gives
                    // its value composited onto the underlying one.
                    let composited = |place: Option<usize>| match sample.composite_of(place) {
                        Composite::Replace => None,
                        op => place.map(|k| {
                            let value = values[k].as_ref().expect("computed above");
                            let mut out = value.clone();
                            animate::composite(longhand, &underlying, value, op, &mut out);
                            out
                        }),
                    };
                    let (first, second) = match read {
                        Read::At(place) => (composited(place), None),
                        Read::Mix { from, to, .. } => (composited(from), composited(to)),
                    };
                    let value = |place: Option<usize>| match place {
                        Some(k) => values[k].as_ref().expect("computed above"),
                        None => &underlying,
                    };
                    match read {
                        Read::At(None) => continue,
                        Read::At(place) => animate::copy(
                            longhand,
                            first.as_ref().unwrap_or_else(|| value(place)),
                            style,
                        ),
                        Read::Mix { from, to, p } => animate::blend(
                            longhand,
                            first.as_ref().unwrap_or_else(|| value(from)),
                            second.as_ref().unwrap_or_else(|| value(to)),
                            p,
                            style,
                        ),
                    }
                    if let Some(side) = border_color_side(longhand) {
                        border_color_set[side] = true;
                    }
                }
            }
        }
    }
}

/// A registered custom property's value at a sample, between two of its
/// keyframes' values, `None` for an implicit keyframe's, the underlying
/// value.
pub(super) struct TypedMix<'a> {
    pub(super) name: &'a str,
    pub(super) from: Option<&'a [Token]>,
    pub(super) to: Option<&'a [Token]>,
    pub(super) p: f64,
}

/// The side whose border colour `longhand` is, if it is one.
fn border_color_side(longhand: Longhand) -> Option<usize> {
    match longhand {
        Longhand::BorderTopColor => Some(TOP),
        Longhand::BorderRightColor => Some(RIGHT),
        Longhand::BorderBottomColor => Some(BOTTOM),
        Longhand::BorderLeftColor => Some(LEFT),
        _ => None,
    }
}

/// Gives the border colours of `style` that no declaration set,
/// `currentcolor`, the colour `color`.
fn current_color(style: &mut ComputedStyle, set: &[bool; 4], color: Color) {
    for (side, &set) in set.iter().enumerate() {
        if !set {
            style.border_color[side] = Color::CURRENT.resolve(color);
        }
    }
}

/// Keyframe `k` of `sample` computed for the element: `underlying` with
/// the keyframe's declarations applied as the element's own would be,
/// but for the font size's and the custom properties', which their own
/// passes take, and a revert keyword's, which takes the value of
/// `reverted`, the style of the user agent's declarations. Past the
/// colour's pass, `color` is the animated colour, which the keyframe's
/// `currentcolor` is.
fn keyframe_style(
    sample: &Sampled<'_>,
    k: usize,
    (underlying, reverted): (&ComputedStyle, Option<&ComputedStyle>),
    parent: &ComputedStyle,
    bases: &LengthBases<'_>,
    color: Option<Color>,
) -> ComputedStyle {
    let mut style = underlying.clone();
    let mut set = [true; 4];
    for (declaration, identity) in sample.keyframes.declarations(&sample.keyframes()[k]) {
        let name = canonical_name(&declaration.name);
        if custom::is_custom_property_name(name) || name == "font-size" {
            continue;
        }
        if revert_keyword(&declaration.value).is_some() {
            apply_revert(&mut style, name, reverted, parent, &mut set);
            continue;
        }
        match substituted(
            &declaration.value,
            identity,
            name,
            underlying.custom.as_ref(),
            bases.resolution,
        ) {
            Specified::Written(value) => {
                apply_declaration(&mut style, name, value, bases, parent, &mut set);
            }
            Specified::Substituted(value) => {
                if !apply_declaration(&mut style, name, &value, bases, parent, &mut set) {
                    apply_css_wide(&mut style, name, CssWide::Unset, parent, &mut set);
                }
            }
            Specified::Invalid => {
                apply_css_wide(&mut style, name, CssWide::Unset, parent, &mut set);
            }
            Specified::Ignored => {}
        }
    }
    if let Some(color) = color {
        style.color = color;
        current_color(&mut style, &set, color);
    }
    style
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use dom::{Document, NodeId};

    use crate::animation::Animations;
    use crate::cascade::{CascadeContext, Recascade, Styles, Stylesheets, TopLayer};
    use crate::computed::{Color, ComputedStyle, LEFT, Overlay, Size, WillChange};
    use crate::selector_matching::PseudoElement;
    use crate::transform::list_matrix;

    /// A document styled with animations, its timeline at zero unless
    /// moved.
    struct Page {
        document: Document,
        sheets: Stylesheets,
        animations: RefCell<Animations>,
        styles: Styles,
        /// What the top layer declares of its elements' `overlay`.
        top_layer: HashMap<NodeId, Overlay>,
    }

    impl TopLayer for HashMap<NodeId, Overlay> {
        fn overlay(&self, element: NodeId) -> Option<Overlay> {
            self.get(&element).copied()
        }
    }

    impl Page {
        fn new(css: &str, html: &str) -> Page {
            let document = Document::from_node(&html::parse_document(html).expect("never fails"));
            let sheets = Stylesheets::new(vec![css::parse_stylesheet(css).expect("parses")]);
            let mut page = Page {
                document,
                sheets,
                animations: RefCell::new(Animations::new()),
                styles: Styles::default(),
                top_layer: HashMap::new(),
            };
            page.cascade();
            page
        }

        fn context(&self) -> CascadeContext<'_> {
            CascadeContext {
                animations: Some(&self.animations),
                top_layer: Some(&self.top_layer),
                ..CascadeContext::default()
            }
        }

        fn cascade(&mut self) {
            self.styles = self.sheets.cascade(&self.document, &self.context());
        }

        fn node(&self, id: &str) -> NodeId {
            self.document
                .subtree(self.document.root())
                .find(|&n| self.document.attr(n, "id") == Some(id))
                .unwrap_or_else(|| panic!("no #{id}"))
        }

        fn style(&self, id: &str) -> &ComputedStyle {
            self.styles.get(self.node(id)).expect("styled")
        }

        fn pseudo(&self, id: &str, pseudo: PseudoElement) -> &ComputedStyle {
            self.styles
                .pseudo(self.node(id), pseudo)
                .expect("the pseudo-element is styled")
        }

        /// Restyles the document after what was done to it since, and
        /// the elements `animated`, as a frame's style stage or a change
        /// between frames does.
        fn restyle(&mut self, animated: &[NodeId]) {
            let changes = self.document.take_changes();
            let context = CascadeContext {
                animations: Some(&self.animations),
                top_layer: Some(&self.top_layer),
                ..CascadeContext::default()
            };
            self.sheets.restyle_animated(
                &self.document,
                &context,
                &mut self.styles,
                &changes,
                animated,
            );
        }

        /// Gives the element `id` the class `class` and restyles, a
        /// change of style between frames.
        fn class(&mut self, id: &str, class: &str) {
            let node = self.node(id);
            self.document.set_attribute(node, "class", class).unwrap();
            self.restyle(&[]);
        }

        /// A frame at the timeline's time `time`, as the engine draws one.
        fn frame(&mut self, time: f64) {
            let moved = {
                let mut animations = self.animations.borrow_mut();
                animations.set_time(time);
                animations.start_pending();
                animations.sample()
            };
            self.restyle(&moved);
            self.animations.borrow_mut().start_pending();
        }
    }

    /// A colour's channels as the reference browser prints them, alpha
    /// aside, and its alpha, each to two places.
    fn rgba(color: Color) -> [f32; 4] {
        let round = |v: f32| (v * 100.0).round() / 100.0;
        [
            round(color.r * 255.0),
            round(color.g * 255.0),
            round(color.b * 255.0),
            round(color.a),
        ]
    }

    fn width(style: &ComputedStyle, basis: f32) -> f32 {
        style
            .width
            .length_percentage()
            .expect("a length")
            .resolve(basis)
    }

    fn margin_left(style: &ComputedStyle) -> f32 {
        style.margin[LEFT].resolve(0.0, f32::NAN)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    const WIDTHS: &str = ".w { width: 0px; transition: width 1s linear }
        .w.on { width: 100px } .w.far { width: 300px }";

    /// The events queued since the last asked: each element's id, the
    /// event's type, its animation's or transition's name and its elapsed
    /// time, in milliseconds.
    fn events(page: &Page) -> Vec<(String, &'static str, String, i64)> {
        page.animations
            .borrow_mut()
            .take_events()
            .into_iter()
            .map(|e| {
                let id = page
                    .document
                    .attr(e.node, "id")
                    .unwrap_or_default()
                    .to_owned();
                (
                    id,
                    e.kind.name(),
                    e.name.to_string(),
                    (e.elapsed * 1000.0).round() as i64,
                )
            })
            .collect()
    }

    fn ev(
        id: &str,
        kind: &'static str,
        name: &str,
        ms: i64,
    ) -> (String, &'static str, String, i64) {
        (id.to_owned(), kind, name.to_owned(), ms)
    }

    #[test]
    fn animations_queue_their_events_at_each_change_of_phase() {
        let mut page = Page::new(
            "@keyframes k { to { margin-left: 100px } }
             #a { animation: k 1s linear 0.5s 2 }
             #b { animation: k 1s linear -0.3s }
             .run { animation: k 1s linear }",
            "<div id=a></div><div id=b></div><div id=c class=run></div><div id=d class=run></div>",
        );
        page.frame(0.0);
        assert_eq!(
            events(&page),
            vec![
                ev("b", "animationstart", "k", 300),
                ev("c", "animationstart", "k", 0),
                ev("d", "animationstart", "k", 0),
            ]
        );
        page.frame(0.4);
        page.class("d", "");
        assert_eq!(events(&page), vec![ev("d", "animationcancel", "k", 400)]);
        page.frame(0.6);
        assert_eq!(events(&page), vec![ev("a", "animationstart", "k", 0)]);
        page.frame(1.6);
        assert_eq!(
            events(&page),
            vec![
                ev("a", "animationiteration", "k", 1000),
                ev("b", "animationend", "k", 1000),
                ev("c", "animationend", "k", 1000),
            ]
        );
        page.frame(2.6);
        assert_eq!(events(&page), vec![ev("a", "animationend", "k", 2000)]);
        page.frame(3.0);
        assert!(events(&page).is_empty());
        // Slowed down, its current iteration back to the one before: the
        // iteration's boundary is the end of the one it is in now.
        page.class("d", "run");
        page.frame(3.05);
        page.frame(3.6);
        let d = page.node("d");
        page.document
            .set_attribute(d, "style", "animation-iteration-count: infinite")
            .unwrap();
        page.restyle(&[]);
        page.frame(4.7);
        events(&page);
        page.document
            .set_attribute(
                d,
                "style",
                "animation-iteration-count: infinite; animation-duration: 2s",
            )
            .unwrap();
        page.restyle(&[]);
        page.frame(4.8);
        assert_eq!(
            events(&page),
            vec![ev("d", "animationiteration", "k", 2000)]
        );
        page.class("d", "");
        assert_eq!(events(&page), vec![ev("d", "animationcancel", "k", 1750)]);
        // Taken out of the document while it runs: cancelled. Finished,
        // it is cancelled without an event.
        page.class("c", "");
        page.class("c", "run");
        page.frame(5.2);
        page.frame(5.5);
        assert_eq!(events(&page), vec![ev("c", "animationstart", "k", 0)]);
        let c = page.node("c");
        let parent = page.document.parent(c).unwrap();
        page.document.remove_child(parent, c).unwrap();
        page.restyle(&[]);
        assert_eq!(events(&page), vec![ev("c", "animationcancel", "k", 300)]);
    }

    #[test]
    fn transitions_queue_their_events_at_each_change_of_phase() {
        let mut page = Page::new(
            ".t { margin-left: 0px; transition: margin-left 1s linear }
             .t.on { margin-left: 100px }
             .t.late { transition-delay: 0.5s }",
            "<div id=t class=t></div><div id=u class='t late'></div><div id=v class=t></div>\
             <div id=w class=t></div>",
        );
        page.frame(0.0);
        page.class("t", "t on");
        page.class("u", "t late on");
        page.class("v", "t on");
        assert!(events(&page).is_empty(), "none before a frame");
        // A sample before a frame has started them: still none.
        page.animations.borrow_mut().sample();
        assert!(events(&page).is_empty());
        page.frame(1.0);
        assert_eq!(
            events(&page),
            vec![
                ev("t", "transitionrun", "margin-left", 0),
                ev("t", "transitionstart", "margin-left", 0),
                ev("u", "transitionrun", "margin-left", 0),
                ev("v", "transitionrun", "margin-left", 0),
                ev("v", "transitionstart", "margin-left", 0),
            ]
        );
        page.frame(1.5);
        assert_eq!(
            events(&page),
            vec![ev("u", "transitionstart", "margin-left", 0)]
        );
        // Undone half way: cancelled, and the transition back runs.
        page.class("v", "t");
        assert_eq!(
            events(&page),
            vec![ev("v", "transitioncancel", "margin-left", 500)]
        );
        page.frame(1.6);
        assert_eq!(
            events(&page),
            vec![
                ev("v", "transitionrun", "margin-left", 0),
                ev("v", "transitionstart", "margin-left", 0),
            ]
        );
        page.frame(2.0);
        assert_eq!(
            events(&page),
            vec![ev("t", "transitionend", "margin-left", 1000)]
        );
        // On one element, a transition's events before an animation's.
        page.sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                "@keyframes k { to { height: 10px } }
                 .t { margin-left: 0px; transition: margin-left 1s linear }
                 .t.on { margin-left: 100px }
                 .t.late { transition-delay: 0.5s }
                 .t.k { animation: k 1s }",
            )
            .unwrap(),
        ]);
        page.class("w", "t on k");
        page.frame(2.1);
        assert_eq!(
            events(&page),
            vec![
                ev("v", "transitionend", "margin-left", 500),
                ev("w", "transitionrun", "margin-left", 0),
                ev("w", "transitionstart", "margin-left", 0),
                ev("w", "animationstart", "k", 0),
            ]
        );
        page.frame(2.6);
        assert_eq!(
            events(&page),
            vec![ev("u", "transitionend", "margin-left", 1000)]
        );
    }

    #[test]
    fn a_transition_starts_at_the_next_frame_and_runs_from_the_old_value() {
        let mut page = Page::new(WIDTHS, "<div id=a class=w></div>");
        page.frame(0.0);
        page.class("a", "w on");
        // Made between frames, it shows its start until the next one.
        assert_eq!(width(page.style("a"), 0.0), 0.0);
        page.frame(1.0);
        assert_eq!(width(page.style("a"), 0.0), 0.0);
        page.frame(1.5);
        assert_eq!(width(page.style("a"), 0.0), 50.0);
        // A whole cascade with nothing before it applies the transition
        // as it stands, and cancels nothing.
        let context = page.context();
        let fresh = page.sheets.cascade(&page.document, &context);
        assert_eq!(width(fresh.get(page.node("a")).unwrap(), 0.0), 50.0);
        assert_eq!(page.animations.borrow().len(), 1);
        page.frame(2.0);
        assert_eq!(width(page.style("a"), 0.0), 100.0);
        assert!(page.animations.borrow().is_empty(), "ended");
        // Completed, and kept while the style before a change and after
        // it differ in its property, which they no longer do.
        assert_eq!(page.animations.borrow().completed_count(), 1);
        let a = page.node("a");
        page.document
            .set_attribute(a, "style", "color: #f00")
            .unwrap();
        page.restyle(&[]);
        assert_eq!(page.animations.borrow().completed_count(), 0);
    }

    #[test]
    fn interrupted_transitions_reverse_faster_or_go_on_from_where_they_were() {
        let mut page = Page::new(
            &format!(
                "{WIDTHS} .e {{ width: 0px; transition: width 1s ease }} .e.on {{ width: 100px }}"
            ),
            "<div id=r class=w></div><div id=i class=w></div><div id=e class=e></div>",
        );
        page.frame(0.0);
        page.class("r", "w on");
        page.class("i", "w on");
        page.class("e", "e on");
        page.frame(1.0);
        page.frame(1.5);
        assert_eq!(width(page.style("r"), 0.0), 50.0);
        // `ease` half way, as the reference browser has it when the
        // transition is reversed there.
        assert!((width(page.style("e"), 0.0) - 80.2403).abs() < 1e-3);
        // Undone half way: back from 50px in half the time.
        page.class("r", "w");
        // Sent elsewhere half way: from 50px to 300px in the whole time.
        page.class("i", "w far");
        // Undone after the eased half: back in 0.8024 of the time.
        page.class("e", "e");
        page.frame(1.6);
        assert_eq!(width(page.style("r"), 0.0), 50.0);
        assert_eq!(width(page.style("i"), 0.0), 50.0);
        page.frame(1.85);
        assert!((width(page.style("r"), 0.0) - 25.0).abs() < 1e-3);
        assert!((width(page.style("i"), 0.0) - 112.5).abs() < 1e-3);
        page.frame(1.6 + 0.8024);
        assert!(width(page.style("e"), 0.0) < 0.01);
        page.frame(2.1);
        assert_eq!(width(page.style("r"), 0.0), 0.0);
        page.frame(2.6);
        assert_eq!(width(page.style("i"), 0.0), 300.0);
        assert!(page.animations.borrow().is_empty());
    }

    #[test]
    fn a_reversal_of_a_reversal_and_of_a_negative_delay_shorten_as_the_reference_browser_does() {
        let mut page = Page::new(
            &format!(
                "{WIDTHS} .n {{ width: 0px; transition: width 1s linear -0.2s }} \
                 .n.on {{ width: 100px }}"
            ),
            "<div id=r class=w></div><div id=n class=n></div>",
        );
        page.frame(0.0);
        page.class("r", "w on");
        page.class("n", "n on");
        page.frame(1.0);
        page.frame(1.3);
        assert_eq!(width(page.style("n"), 0.0), 50.0);
        // Back from 50px with half the delay and half the time: 40px at
        // once.
        page.class("n", "n");
        page.frame(1.4);
        assert!((width(page.style("n"), 0.0) - 40.0).abs() < 1e-3);
        page.frame(1.5);
        page.class("r", "w");
        page.frame(1.6);
        page.frame(1.85);
        assert!((width(page.style("r"), 0.0) - 25.0).abs() < 1e-3);
        // Reversed again a quarter of the way back: to 100px in three
        // quarters of the time.
        page.class("r", "w on");
        page.frame(1.9);
        page.frame(2.275);
        assert!((width(page.style("r"), 0.0) - 62.5).abs() < 1e-3);
        page.frame(2.65);
        assert_eq!(width(page.style("r"), 0.0), 100.0);
    }

    #[test]
    fn which_properties_transition_and_how() {
        let mut page = Page::new(
            "div { height: 10px }
             #m { margin: 0px; transition: margin 1s linear }
             #m.on { margin: 0px 0px 0px 100px }
             #n { width: 0px; transition: width 1s linear -0.25s }
             #n.on { width: 100px }
             #d { width: 0px; transition: width 1s linear 0.5s }
             #d.on { width: 100px }
             #p { position: static; transition: position 1s linear }
             #p.on { position: relative }
             #q { position: static; transition: position 1s linear allow-discrete }
             #q.on { position: relative }
             #v { --x: 10px; width: var(--x); transition: --x 1s allow-discrete }
             #v.on { --x: 50px }
             #all { --x: 10px; width: var(--x); transition: all 1s allow-discrete }
             #all.on { --x: 50px }
             #t { width: 0px; transition: width 1s linear }
             #t.on { width: 100px }
             #t.off { width: 100px; transition-property: height }
             #z { width: 0px; transition: width 0s, height 1s }
             #z.on { width: 100px }
             #vn { --x: 10px; width: var(--x); transition: --x 1s }
             #vn.on { --x: 50px }
             #ad { width: auto; transition: width 1s linear allow-discrete }
             #ad.on { width: 100px }",
            "<div id=m></div><div id=n></div><div id=d></div><div id=p></div>\
             <div id=q></div><div id=v></div><div id=all></div><div id=t></div>\
             <div id=z></div><div id=vn></div><div id=ad></div>",
        );
        page.frame(0.0);
        for id in ["m", "n", "d", "p", "q", "v", "all", "t", "z", "vn", "ad"] {
            page.class(id, "on");
        }
        // One each: `margin`'s changed longhand, `width` for `#n`, `#d`,
        // `#all`, `#t` and `#ad`, `position` with `allow-discrete`, `--x`
        // with it; none for a zero duration, nor for `--x` without it.
        assert_eq!(page.animations.borrow().len(), 8);
        assert_eq!(width(page.style("z"), 0.0), 100.0);
        assert_eq!(width(page.style("vn"), 0.0), 50.0);
        page.frame(1.0);
        // A shorthand named transitions the longhands that changed.
        let m = page.node("m");
        assert!(page.animations.borrow().transitions_of((m, None)));
        // A negative delay starts part way; a positive one shows the
        // start value through it.
        assert_eq!(width(page.style("n"), 0.0), 25.0);
        page.frame(1.25);
        assert_eq!(width(page.style("d"), 0.0), 0.0);
        // A discrete change transitions only with `allow-discrete`, and
        // flips half way.
        assert_eq!(
            page.style("p").position,
            crate::computed::Position::Relative
        );
        assert_eq!(page.style("q").position, crate::computed::Position::Static);
        // A custom property named transitions, discretely; `all` does not
        // name one, though the width that takes it transitions.
        assert_eq!(width(page.style("v"), 0.0), 10.0);
        let x = |id: &str| {
            let style = page.style(id);
            let value = style.custom.as_ref().and_then(|m| m.get("--x")).cloned();
            value.map(|tokens| tokens.to_vec())
        };
        assert_eq!(x("all"), Some(css::tokenize("50px")));
        assert!((width(page.style("all"), 0.0) - 26.3404).abs() < 1e-3);
        // A pair that does not interpolate transitions discretely with
        // `allow-discrete`.
        assert_eq!(page.style("ad").width, Size::Auto);
        page.frame(1.5);
        assert_eq!(width(page.style("ad"), 0.0), 100.0);
        assert_eq!(margin_left(page.style("m")), 50.0);
        assert_eq!(
            page.style("q").position,
            crate::computed::Position::Relative
        );
        assert_eq!(width(page.style("v"), 0.0), 50.0);
        assert_eq!(width(page.style("t"), 0.0), 50.0);
        // `transition-property` no longer naming it: the end at once.
        page.class("t", "off");
        assert_eq!(width(page.style("t"), 0.0), 100.0);
        let t = page.node("t");
        assert!(!page.animations.borrow().transitions_of((t, None)));
    }

    #[test]
    fn transitioned_values_inherit_and_a_childs_own_transition_starts_once() {
        let mut page = Page::new(
            "#p { color: #000; transition: color 4s linear }
             #p.on { color: #c80000 }
             #c { transition: color 1s linear }",
            "<div id=p><span id=c>c</span><em id=e>e</em></div>",
        );
        page.frame(0.0);
        page.class("p", "on");
        page.frame(1.0);
        page.frame(1.5);
        // The child without a transition takes the parent's value; the
        // one with its own runs it from black to the end, once.
        assert_eq!(rgba(page.style("p").color), [25.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgba(page.style("e").color), [25.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgba(page.style("c").color), [100.0, 0.0, 0.0, 1.0]);
        page.frame(2.5);
        // Its own ended, it inherits the parent's again, and no second
        // transition starts for it.
        assert_eq!(rgba(page.style("c").color), [75.0, 0.0, 0.0, 1.0]);
        let c = page.node("c");
        assert!(!page.animations.borrow().transitions_of((c, None)));
        page.frame(3.0);
        assert!(!page.animations.borrow().transitions_of((c, None)));
        assert_eq!(page.animations.borrow().len(), 1);
        page.frame(5.0);
        assert_eq!(rgba(page.style("c").color), [200.0, 0.0, 0.0, 1.0]);
        assert!(page.animations.borrow().is_empty());
    }

    #[test]
    fn an_animation_outranks_a_transition_and_starts_none() {
        let mut page = Page::new(
            "@keyframes grow { from { width: 0px } to { width: 100px } }
             #a { width: 10px; transition: width 1s linear; animation: grow 2s linear }
             #a.on { width: 50px }
             #a.off { width: 50px; animation: none }
             #b { width: 10px; transition: width 1s linear }
             #b.on { width: 50px; animation: grow 2s linear paused -1s }",
            "<div id=a></div><div id=b></div>",
        );
        page.frame(0.0);
        page.frame(0.5);
        page.class("a", "on");
        page.class("b", "on");
        // No transition for a property an animation animates: the
        // animation's value.
        assert_eq!(width(page.style("a"), 0.0), 25.0);
        assert_eq!(width(page.style("b"), 0.0), 50.0);
        assert_eq!(page.animations.borrow().len(), 2, "the two animations");
        // Nor when the animation goes: the value jumps.
        page.class("a", "off");
        assert_eq!(width(page.style("a"), 0.0), 50.0);
        assert_eq!(page.animations.borrow().len(), 1);
    }

    #[test]
    fn a_running_transition_yields_to_an_animation_of_its_property() {
        let mut page = Page::new(
            &format!(
                "{WIDTHS} @keyframes grow {{ from {{ width: 0px }} to {{ width: 100px }} }}
                 .w.on.a {{ animation: grow 2s linear paused -0.4s }}"
            ),
            "<div id=c class=w></div>",
        );
        page.frame(0.0);
        page.class("c", "w on");
        page.frame(1.0);
        page.frame(1.5);
        assert_eq!(width(page.style("c"), 0.0), 50.0);
        page.class("c", "w on a");
        assert_eq!(width(page.style("c"), 0.0), 20.0);
        let c = page.node("c");
        assert!(page.animations.borrow().transitions_of((c, None)));
    }

    #[test]
    fn values_transition_as_computed_and_inherit_and_an_unrendered_element_starts_none() {
        let mut page = Page::new(
            "#b { border-left: 2px solid #f00; color: #00f; transition: border-left-color 1s linear }
             #b.on { border-left: 2px solid }
             #f { font-size: 10px; width: 10em; transition: font-size 1s linear }
             #f.on { font-size: 30px }
             #h { display: none; width: 0px; transition: width 1s linear }
             #h.on { display: block; width: 100px }",
            "<div id=b></div><div id=f></div><div id=h></div>",
        );
        page.frame(0.0);
        page.class("b", "on");
        page.class("f", "on");
        page.class("h", "on");
        // Not rendered before the change: nothing to transition from.
        assert_eq!(width(page.style("h"), 0.0), 100.0);
        assert_eq!(page.animations.borrow().len(), 2);
        page.frame(1.0);
        page.frame(1.5);
        // To `currentcolor`, the colour of the element.
        assert_eq!(
            rgba(page.style("b").border_color[LEFT]),
            [127.5, 0.0, 127.5, 1.0]
        );
        // A length in `em` follows the transitioned font size.
        assert_eq!(page.style("f").font_size_px, 20.0);
        assert_eq!(width(page.style("f"), 0.0), 200.0);
    }

    #[test]
    fn display_transitions_out_with_allow_discrete_and_is_inert_meanwhile() {
        let mut page = Page::new(
            "#a { opacity: 1; transition: opacity 1s linear, display 1s allow-discrete }
             #a.hidden { display: none; opacity: 0 }
             #b { opacity: 1; transition: opacity 1s linear }
             #b.hidden { display: none; opacity: 0 }",
            "<div id=a><span id=s>s</span></div><div id=b></div>",
        );
        page.frame(0.0);
        page.class("a", "hidden");
        page.class("b", "hidden");
        page.frame(1.0);
        page.frame(1.5);
        let a = page.style("a");
        assert_eq!(a.display, crate::computed::Display::Block);
        assert_eq!(a.effects().opacity, 0.5);
        assert!(a.inert && page.style("s").inert, "inert, and what is in it");
        // Without `display` transitioning, it is gone at once and its
        // transitions with it.
        assert_eq!(page.style("b").display, crate::computed::Display::None);
        let b = page.node("b");
        assert!(!page.animations.borrow().transitions_of((b, None)));
        page.frame(2.0);
        assert_eq!(page.style("a").display, crate::computed::Display::None);
        assert!(!page.style("a").inert);
        assert!(page.animations.borrow().is_empty());
    }

    #[test]
    fn a_font_variant_longhand_transitions_discretely_when_it_changes() {
        let mut page = Page::new(
            "#a { transition: all 1s allow-discrete }
             #a.on { font-variant-caps: small-caps }",
            "<div id=a></div>",
        );
        page.frame(0.0);
        page.class("a", "on");
        assert_eq!(page.animations.borrow().len(), 1);
        page.frame(0.25);
        assert_eq!(page.style("a").font().variant.caps, "normal");
        page.frame(0.75);
        assert_eq!(page.style("a").font().variant.caps, "small-caps");
    }

    #[test]
    fn transitions_end_with_the_element_rendered_and_a_whole_cascade_starts_them() {
        let mut page = Page::new(
            WIDTHS,
            "<section id=s><div id=a class=w></div></section><div id=b class=w></div>",
        );
        page.frame(0.0);
        page.class("a", "w on");
        page.class("b", "w on");
        page.frame(1.0);
        page.frame(1.5);
        assert_eq!(page.animations.borrow().len(), 2);
        // Its parent not displayed: no longer rendered.
        let s = page.node("s");
        page.document
            .set_attribute(s, "style", "display: none")
            .unwrap();
        page.restyle(&[]);
        assert_eq!(page.animations.borrow().len(), 1);
        // Shown again, it starts nothing: it was not rendered before.
        page.document.remove_attribute(s, "style").unwrap();
        page.restyle(&[]);
        assert_eq!(page.animations.borrow().len(), 1);
        // Taken out of the document.
        let b = page.node("b");
        let parent = page.document.parent(b).unwrap();
        page.document.remove_child(parent, b).unwrap();
        page.restyle(&[]);
        assert!(page.animations.borrow().is_empty());
        // A change of the sheets, the document cascaded whole from the
        // styles it had.
        page.sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                ".w { width: 0px; transition: width 1s linear } .w.on { width: 200px }",
            )
            .unwrap(),
        ]);
        let context = CascadeContext {
            animations: Some(&page.animations),
            ..CascadeContext::default()
        };
        page.styles = page
            .sheets
            .cascade_from(&page.document, &context, &page.styles);
        assert_eq!(width(page.style("a"), 0.0), 100.0);
        page.frame(2.0);
        page.frame(2.5);
        assert_eq!(width(page.style("a"), 0.0), 150.0);
    }

    #[test]
    fn a_paused_animation_shows_its_moment_as_the_reference_browser_does() {
        let page = Page::new(
            "@keyframes c { from { color: #ff0000 } to { color: #0000ff00 } }
             @keyframes w { from { width: 10px } to { width: 50% } }
             @keyframes z { from { z-index: 1 } to { z-index: 2 } }
             @keyframes t { from { transform: scale(2) rotate(0deg) }
                            to { transform: rotate(90deg) translate(10px) } }
             @keyframes s { from { margin-left: 0px } to { margin-left: 100px } }
             #a { animation: c 1s linear -0.5s paused }
             #b { animation: w 1s linear -0.5s paused; position: absolute }
             #c { animation: z 1s linear -0.5s paused; position: relative }
             #d { animation: t 1s linear -0.5s paused }
             #e { animation: s 1s steps(4) -0.5s paused }
             #f { animation: s 1s steps(4, jump-start) 1s backwards paused }
             #g { animation: s 1s cubic-bezier(0.1, 0.7, 1.0, 0.1) -0.3s paused }",
            "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>\
             <div id=e></div><div id=f></div><div id=g></div>",
        );
        // Red to transparent blue half way is half-transparent red, the
        // alpha premultiplied: `rgba(255, 0, 0, 0.5)`.
        assert_eq!(rgba(page.style("a").color), [255.0, 0.0, 0.0, 0.5]);
        // 10px to 50% of 800px: 205px.
        assert!(close(width(page.style("b"), 800.0), 205.0));
        assert_eq!(page.style("c").z_index, Some(2));
        let matrix = list_matrix(&page.style("d").effects().transform, 10.0, 10.0)
            .to_2d()
            .expect("2D");
        let expected = [1.06066, 1.06066, -1.06066, 1.06066, 0.0, 5.0];
        for (got, want) in matrix.iter().zip(expected) {
            assert!((got - want).abs() < 1e-4, "{matrix:?}");
        }
        assert_eq!(margin_left(page.style("e")), 50.0);
        // The before flag: a jump at the start does not happen in a
        // backwards-filled delay.
        assert_eq!(margin_left(page.style("f")), 0.0);
        assert!((margin_left(page.style("g")) - 36.9156).abs() < 1e-3);
    }

    #[test]
    fn the_font_properties_animate() {
        let page = Page::new(
            "@keyframes s { from { font-stretch: 100% } to { font-stretch: 200% } }
             @keyframes f { from { font-family: serif } to { font-family: monospace } }
             @keyframes v { from { font-variant-caps: normal } to { font-variant-caps: small-caps } }
             #a { animation: s 1s linear -0.5s paused }
             #b { animation: f 1s linear -0.6s paused }
             #c { animation: v 1s linear -0.6s paused }",
            "<div id=a></div><div id=b></div><div id=c></div>",
        );
        assert_eq!(page.style("a").font().stretch, 150.0);
        assert_eq!(&*page.style("b").font().family, "monospace");
        assert!(page.style("c").font().small_caps());
    }

    #[test]
    fn animations_outrank_normal_declarations_and_yield_to_important_ones() {
        let page = Page::new(
            "@keyframes s { from { margin-left: 0px } to { margin-left: 100px } }
             @keyframes w5 { from { width: 0px !important } to { width: 100px } }
             div { animation: s 1s linear -0.5s paused }
             #a { margin-left: 7px !important }
             #c { width: 10px; animation: w5 1s linear -0.5s paused }",
            "<div id=a></div><div id=b style='margin-left: 3px'></div>\
             <div id=c></div><div id=d style='margin-left: 3px !important'></div>",
        );
        assert_eq!(margin_left(page.style("a")), 7.0);
        assert_eq!(margin_left(page.style("b")), 50.0);
        assert_eq!(margin_left(page.style("d")), 3.0);
        // The keyframe's `!important` declaration is ignored: the width
        // runs from the underlying 10px.
        assert_eq!(width(page.style("c"), 0.0), 55.0);
    }

    #[test]
    fn keyframes_are_computed_against_the_animated_element() {
        let page = Page::new(
            "@keyframes f { from { font-size: 10px } to { font-size: 30px } }
             @keyframes w { from { width: 10em } to { width: 10em } }
             @keyframes v { from { --x: 10px; } to { --x: 50px; } }
             @keyframes cc { from { border-left-color: currentcolor } to { border-left-color: #0000c8 } }
             @keyframes c { from { color: #c80000 } to { color: #00c800 } }
             @keyframes w2 { to { width: 110px } }
             @keyframes w3 { 50% { width: 110px } 50% { height: 7px } }
             @keyframes w4 { 0% { width: 0px; animation-timing-function: steps(2) }
                             50% { width: 100px } 100% { width: 300px } }
             @keyframes k { from { color: #f00; text-decoration-color: currentcolor }
                            to { color: #f00; text-decoration-color: #00f } }
             @keyframes g { from { color: #0f0 } to { color: #0f0 } }
             @keyframes f1 { from { width: 0px } }
             @keyframes fs { from { font-size: 10px; font-size: 20px } to { font-size: 20px } }
             #a { animation: f 1s linear paused -0.5s, w 1s linear paused -0.5s }
             #b { width: var(--x); animation: v 1s linear paused -0.4s }
             #c { width: var(--x); animation: v 1s linear paused -0.6s }
             #d { color: #000; border-left: 2px solid; animation: c 1s linear paused -0.5s,
                  cc 1s linear paused -0.5s }
             #g { width: 10px; animation: w2 1s linear paused -0.5s }
             #h { width: 10px; animation: w3 1s linear paused -0.5s }
             #i { width: 10px; animation: w4 1s linear paused -0.25s }
             #j { width: 10px; animation: w4 1s linear paused -0.2s }
             #t { color: #000; text-decoration: underline;
                  animation: k 1s linear paused -0.5s, g 1s linear paused -0.5s }
             #f { width: 100px; animation: f1 1s linear paused -0.5s }
             #s { animation: fs 1s linear paused -0.5s }
             #u { color: #000; background-color: currentcolor; animation: c 1s linear paused -0.5s }",
            "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>\
             <div id=g></div><div id=h></div><div id=i></div><div id=j></div>\
             <div id=t></div><div id=f></div><div id=s></div><div id=u></div>",
        );
        // Gap batch E: another property's `currentcolor` is the colour of
        // the moment too.
        assert_eq!(
            rgba(page.style("u").background_color),
            [100.0, 100.0, 0.0, 1.0]
        );
        // `10em` of the animated font size, 20px.
        assert_eq!(page.style("a").font_size_px, 20.0);
        assert_eq!(width(page.style("a"), 0.0), 200.0);
        // A custom property steps at half way, and `var()` reads it.
        assert_eq!(width(page.style("b"), 0.0), 10.0);
        assert_eq!(width(page.style("c"), 0.0), 50.0);
        // The border's `currentcolor` is the colour of the moment.
        let d = page.style("d");
        assert_eq!(rgba(d.color), [100.0, 100.0, 0.0, 1.0]);
        assert_eq!(rgba(d.border_color[LEFT]), [50.0, 50.0, 100.0, 1.0]);
        // An implicit first keyframe is the underlying value.
        assert_eq!(width(page.style("g"), 0.0), 60.0);
        // Two blocks at one offset are one keyframe.
        assert_eq!(width(page.style("h"), 0.0), 110.0);
        assert_eq!(
            page.style("h")
                .height
                .length_percentage()
                .unwrap()
                .resolve(0.0),
            7.0
        );
        // A keyframe's own easing: `steps(2)` half way to its next
        // keyframe, and below it.
        assert_eq!(width(page.style("i"), 0.0), 50.0);
        assert_eq!(width(page.style("j"), 0.0), 0.0);
        // A keyframe's `currentcolor` is the colour of the moment, which
        // another animation gives, not the keyframe's own.
        let t = page.style("t");
        assert_eq!(rgba(t.color), [0.0, 255.0, 0.0, 1.0]);
        let decoration = t.decoration().color.expect("a colour");
        assert_eq!(rgba(decoration), [0.0, 127.5, 127.5, 1.0]);
        // An implicit last keyframe is the underlying value too.
        assert_eq!(width(page.style("f"), 0.0), 50.0);
        // The last of a keyframe's declarations of a property wins.
        assert_eq!(page.style("s").font_size_px, 20.0);
    }

    #[test]
    fn before_after_and_marker_animate_and_the_first_letter_and_line_do_not() {
        let page = Page::new(
            "@keyframes c { from { color: #000 } to { color: #c86400 } }
             @keyframes w { from { width: 0px } to { width: 100px } }
             p::first-letter { animation: c 1s linear paused -0.5s }
             q::first-line { animation: c 1s linear paused -0.5s }
             div::before { content: 'x'; animation: c 1s linear paused -0.5s }
             li::marker { animation: c 1s linear paused -0.5s }
             span::after { content: 'y'; display: block; animation: w 1s linear paused -0.5s }
             li.m::marker { animation: w 1s linear paused -0.5s }
             em::before { animation: c 1s linear paused -0.5s }
             .on::before { content: 'z'; animation: c 1s linear paused -0.5s }",
            "<p id=p>Hello</p><q id=q style='display: block'>Line</q><div id=d>D</div>\
             <ul><li id=l>A<li id=m class=m>B</ul><span id=s></span><em id=e>E</em>\
             <b id=n class=on>N</b>",
        );
        let half = [100.0, 50.0, 0.0, 1.0];
        assert_eq!(rgba(page.pseudo("d", PseudoElement::Before).color), half);
        assert_eq!(rgba(page.pseudo("l", PseudoElement::Marker).color), half);
        assert_eq!(width(page.pseudo("s", PseudoElement::After), 0.0), 50.0);
        let black = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(
            rgba(page.pseudo("p", PseudoElement::FirstLetter).color),
            black
        );
        assert_eq!(
            rgba(page.pseudo("q", PseudoElement::FirstLine).color),
            black
        );
        // A marker takes only the properties that apply to it.
        assert_eq!(page.pseudo("m", PseudoElement::Marker).width, Size::Auto);
        // The reference browser's four and the last one's; a `::before`
        // without content has no box and no animation.
        assert_eq!(page.animations.borrow().len(), 5);
        assert_eq!(
            rgba(page.pseudo("e", PseudoElement::Before).color),
            black,
            "no content, no animation"
        );
        // A pseudo-element no rule styles any more has no animation.
        let mut page = page;
        let n = page.node("n");
        page.document.remove_attribute(n, "class").unwrap();
        page.cascade();
        assert_eq!(page.animations.borrow().len(), 4);
        assert!(
            !page
                .animations
                .borrow()
                .animates((n, Some(PseudoElement::Before)))
        );
    }

    #[test]
    fn display_none_and_missing_keyframes_run_nothing_until_they_change() {
        let mut page = Page::new(
            "@keyframes s { from { margin-left: 0px } to { margin-left: 100px } }
             .run { animation: s 1s linear paused -0.5s }
             .gone { display: none }",
            "<div id=a class=run></div><div class=gone><div id=b class=run></div></div>\
             <div id=c class='run gone'></div>\
             <div id=d style='animation: nothing 1s'></div>",
        );
        assert_eq!(margin_left(page.style("a")), 50.0);
        // Inside an element not displayed, or not displayed itself.
        assert_eq!(margin_left(page.style("b")), 0.0);
        assert_eq!(margin_left(page.style("c")), 0.0);
        assert_eq!(page.animations.borrow().len(), 1);
        // Hiding the element cancels its animation; showing it again
        // starts a new one.
        let a = page.node("a");
        page.document.set_attribute(a, "class", "run gone").unwrap();
        page.cascade();
        assert!(page.animations.borrow().is_empty());
        page.document.set_attribute(a, "class", "run").unwrap();
        page.cascade();
        assert_eq!(page.animations.borrow().len(), 1);
        // A rule of the name that was missing starts its animation.
        page.sheets = Stylesheets::new(vec![
            css::parse_stylesheet(
                "@keyframes s { from { margin-left: 0px } to { margin-left: 100px } }
                 @keyframes nothing { to { margin-left: 10px } }
                 .run { animation: s 1s linear paused -0.5s }
                 .gone { display: none }",
            )
            .unwrap(),
        ]);
        page.cascade();
        assert_eq!(page.animations.borrow().len(), 2);
    }

    #[test]
    fn an_element_taken_out_loses_its_animations_and_one_moved_starts_again() {
        let mut page = Page::new(
            "@keyframes s { from { margin-left: 0px } to { margin-left: 100px } }
             div { animation: s 1s linear }",
            "<section id=x><div id=a></div><div id=b></div></section><section id=y></section>",
        );
        page.animations.borrow_mut().start_pending();
        page.animations.borrow_mut().set_time(0.5);
        let (a, b, y) = (page.node("a"), page.node("b"), page.node("y"));
        page.document.take_changes();
        // A restyle after a move: the moved element starts again, the
        // other goes on.
        page.document.append_child(y, a).unwrap();
        let changes = page.document.take_changes();
        let context = page.context();
        let mut styles = page.styles.clone();
        page.sheets
            .restyle(&page.document, &context, &mut styles, &changes);
        assert_eq!(margin_left(styles.get(a).unwrap()), 0.0);
        let progress = |target| {
            page.animations.borrow().samples(target)[0]
                .progress
                .progress
        };
        assert_eq!(progress((a, None)), Some(0.0));
        assert_eq!(progress((b, None)), Some(0.5));
        // A whole cascade after a removal: the element's animation is
        // gone.
        let parent = page.document.parent(b).unwrap();
        page.document.remove_child(parent, b).unwrap();
        page.cascade();
        assert_eq!(page.animations.borrow().len(), 1);
        assert!(page.animations.borrow().animates((a, None)));
    }

    #[test]
    fn the_keyframes_in_force_are_the_last_layers_then_the_last_rules() {
        let page = Page::new(
            "@layer one, two;
             @keyframes k { to { margin-left: 100px } }
             @layer two { @keyframes k { to { margin-left: 200px } } }
             @keyframes j { to { margin-left: 100px } }
             @-webkit-keyframes j { to { margin-left: 300px } }
             @media print { @keyframes j { to { margin-left: 400px } } }
             @layer one { @keyframes l { to { margin-left: 100px } } }
             @layer two { @keyframes l { to { margin-left: 500px } } }
             #k { animation: k 1s linear paused -0.5s }
             #j { animation: j 1s linear paused -0.5s }
             #l { animation: l 1s linear paused -0.5s }",
            "<div id=k></div><div id=j></div><div id=l></div>",
        );
        // Unlayered rules come last in the order of normal declarations.
        assert_eq!(margin_left(page.style("k")), 50.0);
        // The last rule of the name, its `-webkit-` form among them, and
        // not one whose medium is not this one.
        assert_eq!(margin_left(page.style("j")), 150.0);
        assert_eq!(margin_left(page.style("l")), 250.0);
    }

    #[test]
    fn a_custom_property_an_animation_sets_is_tainted_for_properties_not_animatable() {
        let page = Page::new(
            "@keyframes v { from { --d: 2s; --w: 10px } to { --d: 2s; --w: 10px } }
             #a { animation: v 1s linear paused }
             #a, #b { --e: var(--d); width: var(--w); transition-duration: var(--d, 3s) }
             #b { animation-delay: var(--e, 4s) }",
            "<div id=a><div id=b></div></div>",
        );
        for id in ["a", "b"] {
            let style = page.style(id);
            // Animatable: the animated value.
            assert_eq!(width(style, 0.0), 10.0, "#{id}");
            // Not animatable: the fallback, the tainted value being the
            // guaranteed-invalid value there, and a property that took
            // it tainted too.
            assert_eq!(style.motion().transition_duration, vec![3.0], "#{id}");
            let custom = style.custom.as_ref().expect("custom properties");
            assert!(
                custom.is_tainted("--d") && custom.is_tainted("--e"),
                "#{id}"
            );
        }
        assert_eq!(page.style("b").motion().animation_delay, vec![4.0]);
        assert_eq!(page.animations.borrow().len(), 1);
    }

    #[test]
    fn a_transitioned_custom_property_reads_an_attribute_as_its_value_did() {
        // Values 5 section 7.7.3: an attribute's value never becomes a
        // URL, through a transition of the property holding it neither,
        // registered or not; a value that read none stays a URL.
        let mut page = Page::new(
            "@property --s { syntax: '<string>'; inherits: false; initial-value: 'x' }
             @property --n { syntax: '<length>'; inherits: false; initial-value: 0px }
             div { --s: 'a.png'; --u: 'a.png'; --n: 0px; background-image: image-set(var(--s));
                   transition: --s 1s linear allow-discrete, --u 1s linear allow-discrete,
                               --n 1s linear }
             #u { background-image: image-set(var(--u)) }
             div.on { --s: attr(data-u); --u: attr(data-u); --n: attr(data-n px) }
             #back { --s: attr(data-u); --u: attr(data-u) }
             #back.on { --s: 'a.png'; --u: 'a.png' }",
            "<div id=s data-u=b.png data-n=8></div><div id=u data-u=b.png></div>\
             <div id=back data-u=b.png></div>",
        );
        let image = |page: &Page, id: &str| page.style(id).background().image.clone();
        let attr = |page: &Page, id: &str, name: &str| {
            let custom = page.style(id).custom.clone().expect("custom properties");
            custom.is_attr_tainted(name)
        };
        page.frame(0.0);
        assert!(image(&page, "s")[0].is_some());
        assert!(image(&page, "u")[0].is_some());
        assert_eq!(image(&page, "back"), vec![None]);
        for id in ["s", "u", "back"] {
            page.class(id, "on");
        }
        page.frame(1.0);
        page.frame(1.25);
        // Before half way the values stepped from: `#s` and `#u` read no
        // attribute yet, `#back` still does.
        assert!(!attr(&page, "s", "--s") && !attr(&page, "u", "--u"));
        assert!(image(&page, "s")[0].is_some() && image(&page, "u")[0].is_some());
        assert!(attr(&page, "back", "--s") && attr(&page, "back", "--u"));
        assert_eq!(image(&page, "back"), vec![None]);
        // Interpolated, by a registered type, either value's reaches it.
        let n = page.style("s").custom.clone().expect("custom properties");
        assert_eq!(n.get("--n").map(|t| t.to_vec()), Some(css::tokenize("2px")));
        assert!(attr(&page, "s", "--n"));
        page.frame(1.75);
        // Past it, the values stepped to.
        for id in ["s", "u"] {
            assert_eq!(image(&page, id), vec![None], "#{id}");
        }
        assert!(attr(&page, "u", "--u") && !attr(&page, "back", "--u"));
        assert!(page.animations.borrow().len() >= 6);
    }

    #[test]
    fn a_transition_waiting_out_its_delay_wants_a_frame_at_its_end() {
        // A transition of no duration and a delay changes nothing until
        // the delay ends, when it ends and is reported: a frame is wanted
        // then, and none before.
        let mut page = Page::new(
            "#d { width: 0px; transition: width 0s linear 5s }
             #d.on { width: 100px }
             #e { width: 0px; transition: width 1s linear 2s }
             #e.on { width: 100px }",
            "<div id=d></div><div id=e></div>",
        );
        page.frame(0.0);
        page.class("d", "on");
        page.frame(1.0);
        page.frame(1.016);
        assert!(!page.animations.borrow().needs_frame());
        let next = page
            .animations
            .borrow()
            .next_change()
            .expect("a change ahead");
        assert!((next - 6.0).abs() < 1e-6, "{next}");
        page.class("e", "on");
        page.frame(1.032);
        page.frame(1.048);
        let next = page
            .animations
            .borrow()
            .next_change()
            .expect("a change ahead");
        assert!((next - 3.032).abs() < 1e-6, "{next}");
    }

    #[test]
    fn a_transition_of_overlay_keeps_an_element_leaving_the_top_layer_in_it_until_it_ends() {
        // CSS Position 4 sections 3.2 and 3.3: only the user agent sets
        // `overlay`, as the top layer declares it; an author's own
        // declaration is ignored. With `allow-discrete` the value stays
        // `auto` through the transition and is `none` at its end; without
        // it the removal is at once.
        let mut page = Page::new(
            "div { overlay: auto !important }
             #t { transition: overlay 1s linear allow-discrete }
             #n { transition: overlay 1s linear }
             @keyframes o { from { overlay: auto } to { overlay: auto } }
             #k { animation: o 1s linear paused }",
            "<div id=t></div><div id=n></div><div id=a></div><div id=k></div>",
        );
        let overlay = |page: &Page, id: &str| page.style(id).effects().overlay;
        assert_eq!(overlay(&page, "a"), Overlay::None);
        // An animation sets it, as the reference browser lets one.
        page.frame(0.0);
        page.frame(0.016);
        assert_eq!(overlay(&page, "k"), Overlay::Auto);
        let (t, n) = (page.node("t"), page.node("n"));
        page.frame(0.0);
        page.top_layer.insert(t, Overlay::Auto);
        page.top_layer.insert(n, Overlay::Auto);
        page.restyle(&[t, n]);
        // Into the top layer: `auto` as soon as the transition from
        // `none` runs, strictly between its ends the value it shows.
        page.frame(0.0);
        page.frame(0.25);
        assert_eq!(overlay(&page, "t"), Overlay::Auto);
        assert_eq!(overlay(&page, "n"), Overlay::Auto);
        page.frame(2.0);
        page.top_layer.insert(t, Overlay::None);
        page.top_layer.insert(n, Overlay::None);
        page.restyle(&[t, n]);
        page.frame(2.0);
        assert_eq!(overlay(&page, "n"), Overlay::None);
        page.frame(2.5);
        assert_eq!(overlay(&page, "t"), Overlay::Auto);
        page.frame(2.99);
        assert_eq!(overlay(&page, "t"), Overlay::Auto);
        page.frame(3.0);
        assert_eq!(overlay(&page, "t"), Overlay::None);
    }

    #[test]
    fn an_animated_custom_property_reads_an_attribute_as_its_keyframes_do() {
        // A registered property's keyframes, stepped or interpolated,
        // give their attribute's taint to the value (Values 5 section
        // 7.7.3), and the underlying value its own.
        let page = Page::new(
            "@property --s { syntax: '<string>'; inherits: false; initial-value: 'x' }
             @property --n { syntax: '<length>'; inherits: false; initial-value: 0px }
             @keyframes s { from { --s: 'a.png'; --n: 0px } to { --s: attr(data-u); --n: attr(data-n px) } }
             @keyframes b { to { --s: 'a.png' } }
             div { --s: 'a.png'; background-image: image-set(var(--s)) }
             #early { animation: s 1s linear -0.25s paused }
             #late { animation: s 1s linear -0.75s paused }
             #b { --s: attr(data-u); animation: b 1s linear -0.25s paused }",
            "<div id=early data-u=b.png data-n=8></div><div id=late data-u=b.png data-n=8></div>\
             <div id=b data-u=b.png></div>",
        );
        let custom = |id: &str| page.style(id).custom.clone().expect("custom properties");
        let image = |id: &str| page.style(id).background().image.clone();
        assert!(!custom("early").is_attr_tainted("--s") && image("early")[0].is_some());
        assert!(custom("late").is_attr_tainted("--s"));
        assert_eq!(image("late"), vec![None]);
        // Interpolated, either keyframe's attribute reaches it.
        assert!(custom("early").is_attr_tainted("--n"));
        // Before half way, the underlying value's, which read one.
        assert!(custom("b").is_attr_tainted("--s"));
        assert_eq!(image("b"), vec![None]);
    }

    #[test]
    fn an_animation_acts_as_will_change_while_current_or_in_effect() {
        let mut page = Page::new(
            "@keyframes t { from { transform: rotate(10deg) } to { transform: none } }
             @keyframes o { to { opacity: 0.5 } }
             #a { animation: t 1s linear 5s paused }
             #b { animation: o 1s linear -2s paused }
             #c { animation: o 1s linear -2s paused forwards }",
            "<div id=a></div><div id=b></div><div id=c></div>",
        );
        // In its delay, not filled: no value, but a stacking context, a
        // containing block and a layer all the same.
        let a = page.style("a").clone();
        assert!(a.effects().transform.is_empty());
        assert!(
            a.effects()
                .will_change_in_effect()
                .names(WillChange::TRANSFORM)
        );
        assert!(
            a.effects().will_change.is_auto(),
            "will-change itself is not changed"
        );
        assert!(a.is_stacking_context(false));
        assert!(a.effects().contains_all_descendants());
        // Past its end and not filled: nothing; filled forwards: in effect.
        assert!(page.style("b").effects().will_change_in_effect().is_auto());
        assert!(!page.style("b").is_stacking_context(false));
        assert!(page.style("c").is_stacking_context(false));
        // Gone with the animation.
        page.sheets = Stylesheets::new(Vec::new());
        page.cascade();
        assert!(page.style("a").effects.is_none());
        // Motion properties set back to their initial values leave no
        // copy of them behind.
        page.sheets = Stylesheets::new(vec![
            css::parse_stylesheet("div { animation: none; transition: all 0s }").unwrap(),
        ]);
        page.cascade();
        assert!(page.style("a").motion.is_none());
    }

    #[test]
    fn a_transition_acts_as_will_change_from_its_start_to_its_end() {
        // G33 (Web Animations 1 section 5.6): a transition of a transform
        // from `none` makes a stacking context and a layer from its first
        // frame, and an opacity's from 1 in its delay too;
        // ended, the box is as its style says.
        let mut page = Page::new(
            "#t { transition: transform 1s linear } #t.on { transform: rotate(90deg) }
             #o { transition: opacity 1s linear 0.5s } #o.on { opacity: 0.5 }",
            "<div id=t></div><div id=o></div>",
        );
        page.frame(0.0);
        assert!(!page.style("t").is_stacking_context(false));
        page.class("t", "on");
        page.class("o", "on");
        page.frame(1.0);
        let t = page.style("t").clone();
        assert!(
            t.effects()
                .will_change_in_effect()
                .names(WillChange::TRANSFORM)
        );
        assert!(t.is_stacking_context(false));
        // In its delay, its start value shown: acting all the same.
        let o = page.style("o").clone();
        assert_eq!(o.effects().opacity, 1.0);
        assert!(
            o.effects()
                .will_change_in_effect()
                .names(WillChange::OPACITY)
        );
        page.frame(2.5);
        assert!(page.style("o").effects().will_change_in_effect().is_auto());
        assert!(page.style("t").effects().will_change_in_effect().is_auto());
    }

    #[test]
    fn a_recascade_applies_the_animations_and_starts_none() {
        let page = Page::new(
            "@keyframes s { from { margin-left: 0px } to { margin-left: 100px } }
             @keyframes c { from { color: #000 } to { color: #c86400 } }
             span { animation: s 1s linear paused -0.5s }
             span::before { content: 'b'; animation: c 1s linear paused -0.5s }",
            "<p><span id=s>text</span></p>",
        );
        let context = page.context();
        let recascade = page
            .sheets
            .recascader(&page.document, &page.styles, &context);
        let other = ComputedStyle {
            font_size_px: 30.0,
            ..ComputedStyle::default()
        };
        let span = page.node("s");
        let style = recascade.element(span, &other);
        assert_eq!(style.font_size_px, 30.0);
        assert_eq!(margin_left(&style), 50.0);
        let before = recascade
            .pseudo_element(span, PseudoElement::Before, &other)
            .expect("styled");
        assert_eq!(rgba(before.color), [100.0, 50.0, 0.0, 1.0]);
        assert_eq!(page.animations.borrow().len(), 2);
    }

    /// M35.2: the base styles a cascade keeps for the elements after, and
    /// an animation of composited longhands set on the style made without
    /// it, give every element the style a cascade keeping nothing gives,
    /// frame after frame and across a change: an animation with an
    /// important declaration of its property, one of a colour, a
    /// transition, an `attr()`, an inline style and a `dir` among them.
    #[test]
    fn kept_and_composited_styles_are_those_made_afresh() {
        let css = ".f { animation: fade 1s linear infinite alternate }
            @keyframes fade { to { opacity: 0.2; transform: translateX(30px) rotate(10deg); scale: 1.5 } }
            .i { opacity: 1 !important }
            .c { animation: tint 1s linear infinite }
            @keyframes tint { to { color: rgb(200, 0, 0) } }
            .t { transition: opacity 1s linear, color 1s linear }
            .t.on { opacity: 0.5; color: blue }
            li { margin: 4px; --n: attr(data-n) }";
        let html = "<div id=root><p class=f id=a>a</p><p class=f id=b>b</p>
            <p class='f i' id=c>c</p><p class=c id=d>d</p><p class=t id=e>e</p><ul>
            <li data-n=1>1</li><li data-n=2>2</li><li style='color: green'>3</li>
            <li dir=rtl>4</li><li>5</li><li>6</li></ul></div>";
        let plain = |on: bool| crate::cascade::PLAIN.with(|p| p.set(on));
        let set_on = || crate::cascade::SET_ON.with(std::cell::Cell::get);
        plain(true);
        let mut reference = Page::new(css, html);
        plain(false);
        let mut kept = Page::new(css, html);
        let compare = |reference: &Page, kept: &Page, at: &str| {
            for node in reference.document.subtree(reference.document.root()) {
                assert_eq!(
                    reference.styles.get(node),
                    kept.styles.get(node),
                    "{at}: {node:?}"
                );
            }
        };
        compare(&reference, &kept, "the first cascade");
        let before = set_on();
        for (n, time) in [0.0, 0.1, 0.25, 0.5, 0.9, 1.3].into_iter().enumerate() {
            if n == 2 {
                plain(true);
                reference.class("e", "t on");
                plain(false);
                kept.class("e", "t on");
            }
            plain(true);
            reference.frame(time);
            plain(false);
            kept.frame(time);
            compare(&reference, &kept, &format!("the frame at {time}"));
        }
        assert!(set_on() > before, "the composited animations were set on");
    }
}
