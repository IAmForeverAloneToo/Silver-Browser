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
//! A custom property steps discretely, at half way.

use crate::animate::{self, Longhand};
use crate::animation::{Interval, interval};
use crate::computed::{BOTTOM, Color, ComputedStyle, LEFT, RIGHT, TOP};
use crate::custom;
use crate::easing::Easing;
use crate::keyframes::Keyframe;
use crate::rule_set::KeyframesInForce;

use super::{
    CssWide, LengthBases, Resolution, Specified, Target, apply_css_wide, apply_declaration,
    canonical_name, declared_font_size, substituted,
};

/// A target's animations in effect at the timeline's time, in their
/// order.
pub(super) struct Animated<'a> {
    pub(super) samples: Vec<Sampled<'a>>,
}

/// One animation in effect at the timeline's time.
pub(super) struct Sampled<'a> {
    /// The keyframes it runs.
    pub(super) keyframes: KeyframesInForce<'a>,
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

impl<'a> Sampled<'a> {
    /// The keyframes, in order.
    fn keyframes(&self) -> &'a [Keyframe] {
        &self.keyframes.keyframes.keyframes
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

    /// The last declaration of `name` in keyframe `k`: the one that wins
    /// there, every declaration of a keyframe being valid where it was
    /// parsed.
    fn last_declaration(&self, k: usize, name: &str) -> Option<&'a css::Declaration> {
        let keyframes: &'a [Keyframe] = self.keyframes();
        self.keyframes
            .declarations(&keyframes[k])
            .filter(|d| d.name == name)
            .last()
    }
}

impl<'a> Animated<'a> {
    /// The custom property declarations the animations give, in order,
    /// for the custom properties' pass: each the declaration of the
    /// keyframe its discrete step takes, none where that is an implicit
    /// keyframe, whose value is the underlying one.
    pub(super) fn custom_declarations(&self) -> Vec<custom::Declaration<'a>> {
        let mut out = Vec::new();
        for sample in &self.samples {
            for name in &sample.keyframes.keyframes.custom {
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
                    && let Some(declaration) = sample.last_declaration(k, name)
                {
                    out.push(custom::Declaration {
                        name: &declaration.name,
                        value: &declaration.value,
                        identity: Some(std::ptr::from_ref(declaration) as usize),
                        animated: true,
                    });
                }
            }
        }
        out
    }

    /// Applies the font size the animations give to `style`, which holds
    /// the one from the normal declarations, for the font size's pass: a
    /// keyframe's own `em` and percentage of the parent's, as the
    /// property's always are.
    pub(super) fn font_size(
        &self,
        style: &mut ComputedStyle,
        parent: &ComputedStyle,
        resolution: &Resolution<'_>,
    ) {
        for sample in &self.samples {
            if sample
                .keyframes
                .keyframes
                .longhands
                .binary_search(&Longhand::FontSize)
                .is_err()
            {
                continue;
            }
            let underlying = style.font_size_px;
            let value = |place: Option<usize>| {
                place
                    .and_then(|k| sample.last_declaration(k, "font-size"))
                    .and_then(|declaration| {
                        let identity = Some(std::ptr::from_ref(declaration) as usize);
                        let specified = substituted(
                            &declaration.value,
                            identity,
                            "font-size",
                            style.custom.as_ref(),
                            resolution,
                        );
                        declared_font_size(specified, parent, resolution)
                    })
                    .unwrap_or(underlying)
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
    /// animation sets counts in `border_color_set` as one declared.
    pub(super) fn apply(
        &self,
        style: &mut ComputedStyle,
        parent: &ComputedStyle,
        target: Target,
        bases: &LengthBases<'_>,
        border_color_set: &mut [bool; 4],
    ) {
        for color in [true, false] {
            for sample in &self.samples {
                let longhands: Vec<Longhand> = sample
                    .keyframes
                    .keyframes
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
                                &underlying,
                                parent,
                                bases,
                                (!color).then_some(style.color),
                            ));
                        }
                    }
                    let value = |place: Option<usize>| match place {
                        Some(k) => values[k].as_ref().expect("computed above"),
                        None => &underlying,
                    };
                    match read {
                        Read::At(None) => continue,
                        Read::At(place) => animate::copy(longhand, value(place), style),
                        Read::Mix { from, to, p } => {
                            animate::blend(longhand, value(from), value(to), p, style)
                        }
                    }
                    if let Some(side) = border_color_side(longhand) {
                        border_color_set[side] = true;
                    }
                }
            }
        }
    }
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
            style.border_color[side] = color;
        }
    }
}

/// Keyframe `k` of `sample` computed for the element: `underlying` with
/// the keyframe's declarations applied as the element's own would be,
/// but for the font size's and the custom properties', which their own
/// passes take. Past the colour's pass, `color` is the animated colour,
/// which the keyframe's `currentcolor` is.
fn keyframe_style(
    sample: &Sampled<'_>,
    k: usize,
    underlying: &ComputedStyle,
    parent: &ComputedStyle,
    bases: &LengthBases<'_>,
    color: Option<Color>,
) -> ComputedStyle {
    let mut style = underlying.clone();
    let mut set = [true; 4];
    for declaration in sample.keyframes.declarations(&sample.keyframes()[k]) {
        let name = canonical_name(&declaration.name);
        if custom::is_custom_property_name(name) || name == "font-size" {
            continue;
        }
        let identity = Some(std::ptr::from_ref(declaration) as usize);
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

    use dom::{Document, NodeId};

    use crate::animation::Animations;
    use crate::cascade::{CascadeContext, Recascade, Styles, Stylesheets};
    use crate::computed::{Color, ComputedStyle, LEFT, Size, WillChange};
    use crate::selector_matching::PseudoElement;
    use crate::transform::list_matrix;

    /// A document styled with animations, its timeline at zero unless
    /// moved.
    struct Page {
        document: Document,
        sheets: Stylesheets,
        animations: RefCell<Animations>,
        styles: Styles,
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
            };
            page.cascade();
            page
        }

        fn context(&self) -> CascadeContext<'_> {
            CascadeContext {
                animations: Some(&self.animations),
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
             #s { animation: fs 1s linear paused -0.5s }",
            "<div id=a></div><div id=b></div><div id=c></div><div id=d></div>\
             <div id=g></div><div id=h></div><div id=i></div><div id=j></div>\
             <div id=t></div><div id=f></div><div id=s></div>",
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
}
