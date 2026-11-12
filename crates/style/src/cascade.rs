//! The cascade: origin sorting, specificity, inheritance, and property
//! parsing — turning a [`dom::Document`] plus its stylesheets into
//! [`Styles`], each element's computed style by its [`dom::NodeId`]
//! (roadmap M5, M13).
//!
//! Scope of this milestone, deliberately:
//!
//! * At-rules (M20): the style rules inside `@media`, `@supports` and
//!   `@layer` blocks, to any depth, apply as `crate::rule_set` says:
//!   `@media` lists evaluated against [`CascadeContext::media`] at each
//!   cascade, `@supports` decided once, and the declarations sorted by
//!   their cascade layers. Every other at-rule's rules are passed over.
//! * Colors: hex (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`), CSS Color
//!   4's 148 names (M17; eighteen before), `rgb()` (M25), `hsl()` and
//!   `hwb()` (M30).
//! * Lengths (M10.1): `px`, the absolute units (`in`, `cm`, `mm`, `q`,
//!   `pt`, `pc`), `em`, `rem`, `ex`, `ch`, and the viewport units `vw`,
//!   `vh`, `vmin`, `vmax`. The relative ones resolve here, against the
//!   [`CascadeContext`] the caller gives: the viewport for the viewport
//!   units (zero when none is given, and [`Styles::uses_viewport_units`]
//!   says whether any were met, so a caller can recascade on a viewport
//!   change only when it matters), and a [`FontMeasure`] for `ex` and
//!   `ch` (half an em each without one, the fallbacks css-values-4
//!   allows). Percentages for `width`, `height`, `margin` and `padding`
//!   are kept as written: what they are percentages of is the containing
//!   block's, which only layout knows. `font-size: <percentage>` is of the
//!   parent's font size and resolves here.
//! * `display`: `block`, `inline`, `inline-block` and `none` (M5.1's
//!   own v1 scope, plus M10.4's `inline-block`), the table values of
//!   CSS2 §17.2 (M12), `flex` and `inline-flex` with the Compatibility
//!   Standard's `-webkit-` forms of the two (M17), and `grid` and
//!   `inline-grid` (M18).
//! * Flexbox (M17): `flex-direction`, `flex-wrap`, `flex-flow`, `order`,
//!   `flex-grow`, `flex-shrink`, `flex-basis` and `flex`; the alignment
//!   properties `justify-content`, `align-content`, `align-items` and
//!   `align-self` with the `place-*` shorthands; `row-gap`, `column-gap`
//!   and `gap`. A flex container's children are blockified. The
//!   `-webkit-` aliases the Compatibility Standard requires (§3.4.1) are
//!   the properties under a second name (`canonical_name`).
//! * Grid (M18): `grid-template-columns`, `grid-template-rows` and
//!   `grid-template-areas`, `grid-auto-columns`, `grid-auto-rows` and
//!   `grid-auto-flow`, the placement properties `grid-row-start`,
//!   `grid-row-end`, `grid-column-start` and `grid-column-end`, and the
//!   shorthands `grid-template`, `grid`, `grid-row`, `grid-column` and
//!   `grid-area`, parsed in `cascade::grid`; `justify-items` and
//!   `justify-self`, which `place-items` and `place-self` set too; and
//!   the legacy gap names `grid-row-gap`, `grid-column-gap` and
//!   `grid-gap`. A grid container's children are blockified, as a flex
//!   container's are.
//! * Sizes: `width`, `height` and the minimums and maximums take
//!   css-sizing-3's `min-content`, `max-content` and `fit-content`, and
//!   `min-width` and `min-height` start at `auto` (M17).
//! * `float`, `clear`, `position`, `top`/`right`/`bottom`/`left` and
//!   `z-index` (M11), with CSS2 §9.7's computed-value rules applied
//!   after the longhands: an absolutely positioned box does not float,
//!   and a box that floats or is absolutely positioned computes to the
//!   block-level `display` of §9.7's table
//!   ([`crate::computed::Display::blockified`]), as the root element
//!   always does (M16).
//! * The table properties (M12): `table-layout`, `border-collapse`,
//!   `border-spacing`, `caption-side`, `empty-cells` and
//!   `vertical-align`, the last parsed in full though read on table
//!   cells alone.
//!
//! * Custom properties (M21.1) and the substitution functions `var()`,
//!   `env()` (M21.3) and `attr()` (M21.5), by [`crate::custom`]: an
//!   element's custom properties are computed before its other values,
//!   which substitute them; a value that its property does not take once
//!   substituted is invalid at computed-value time, and the property
//!   `unset`.
//! * The transition and animation properties (M22): `transition-*` and
//!   `animation-*` with their shorthands and the Compatibility
//!   Standard's `-webkit-` names, their easing functions by
//!   [`crate::easing`]; a `::marker` takes them too.
//! * CSS animations (M22.2): with a [`CascadeContext::animations`], an
//!   element's `animation-name` runs the `@keyframes` rules in force by
//!   [`crate::animation`], and their values at the timeline's time apply
//!   between the normal declarations and the important ones
//!   (`cascade::animated`), on the element and on its `::before`,
//!   `::after` and `::marker`.
//! * CSS transitions (M22.1): a restyle, and a whole cascade from the
//!   styles it replaces ([`Stylesheets::cascade_from`]), is a change of
//!   style that starts, replaces and cancels transitions by Transitions 1
//!   §3, each element's style before it against its after-change style,
//!   which inherits from its parent's; their values apply after
//!   everything, but for a property an animation animates. An element
//!   shown by a transition of `display` alone is inert.
//!
//! What every property accepts: the CSS-wide keywords `inherit`,
//! `initial` and `unset` (css-cascade-5 §7.3), resolved against the
//! parent's computed value or the property's initial value. What no
//! length may be: non-finite, or beyond
//! [`crate::cascade::MAX_LENGTH_PX`] in magnitude (a `1e39px` would be
//! infinity in `f32`, and infinity minus infinity is the NaN that puts
//! text at the top-left corner); `font-size`, `padding` and
//! `border-width` may not be negative either.
//!
//! The selectors of every rule are parsed once per sheet, not once per
//! element: a thousand rules against ten thousand elements is ten
//! million matches, which is the work, not ten million parses. The
//! built-in user-agent sheet and its selectors are parsed once per
//! process.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use css::Token;
use dom::{Change, Document, NodeId};

use crate::animate::{self, Longhand};
use crate::animation::{Animations, Property, SampleFrames, Timing, Transitioning, Value, Wanted};
use crate::calc::{self, LengthUnit, Want};
use crate::computed::{
    AlignPosition, BOTTOM, BaselinePosition, BorderCollapse, BorderSpacing, BorderStyle, BoxSizing,
    CaptionSide, Clear, Color, ComputedStyle, Content, ContentAlignment, ContentItem, Cursor,
    Direction, Display, Edges, EmptyCells, FlexBasis, FlexDirection, FlexWrap, Float, LEFT,
    LengthPercentage, LengthPercentageAuto, MarginEdges, MaxSize, Outline, OutlineStyle, Overflow,
    OverflowAlign, PaddingEdges, Position, Quotes, RIGHT, SelfAlignment, Size, TOP, TableLayout,
    TextAlign, TextAlignLast, TextJustify, VerticalAlign, WhiteSpace, WhiteSpaceCollapse,
};
use crate::computed::{
    AnimationPlayState, Backgrounds, BorderImage, Calc, Containment, Decoration, Effects, Filter,
    FontKerning, FontProps, Fragmentation, GridStyle, ImageStyle, InlineSide, JustifyItems, Lists,
    Motion, Overlay, Radii, Resize, ScrollSnapType, SnapAlign, SnapAxis, SnapStrictness, Svg,
    WillChange,
};
use crate::container::{ContainerSizes, Containers, Queried};
use crate::custom::{self, CustomProperties, DataType, Substituted, Substitutions};
use crate::invalidation::{Reach, SelectorFeatures, invalidate};
use crate::media::{ColorScheme, Media};
use crate::page::{PagePosition, PageStyle};
use crate::property::{PropertyRegistration, Registry};
use crate::rule_set::{InForce, KeyframesInForce, RuleSet};
use crate::selector_matching::{
    HighlightName, Matcher, PseudoArgument, PseudoElement, Selector, SelectorList, Specificity,
};

mod animated;
mod backgrounds;
mod border_image;
mod containment;

/// `contain`'s value, if `tokens` are one (CSS Containment 2 section 2).
pub(crate) fn parse_contain_value(tokens: &[Token]) -> Option<crate::computed::Contain> {
    containment::parse_contain(tokens)
}
mod decoration;
mod effects;
mod fonts;
pub use fonts::font_shorthand_families;
pub use fonts::{
    VARIANT_LONGHANDS, font_longhands, parse_variant, stretch_name, variant_field, variant_value,
    write_font_variant,
};
pub(crate) use fonts::{plain_angle, plain_number};
mod fragmentation;
mod gradients;
mod grid;
mod images;
mod lists;
mod logical;
mod motion;
mod page;
mod palette;
pub use palette::{
    PaletteValues, font_palette_specified, font_palette_text, interpolate_font_palette,
    palette_values_name,
};
mod presentational;
mod registered;
mod revert;
#[cfg(test)]
mod scoping_tests;
mod svg;
mod text;
pub mod timelines;

pub use motion::animation_part_texts as motion_animation_parts;
mod top_layer;

use animated::Animated;
pub(crate) use images::{is_frame_hint, is_img_hint};
pub(crate) use lists::is_list_hint;
pub use logical::longhands_of as logical_longhands;

/// The physical longhand a flow-relative longhand is in left-to-right
/// horizontal writing (D74): what its value is written as; `None` for any
/// other property.
pub fn physical_name(name: &str) -> Option<String> {
    logical::physical_longhand(name, Direction::Ltr)
}
pub(crate) use logical::{
    LONGHANDS as LOGICAL_LONGHANDS, Mapped as LogicalMapped, map as logical_map,
};
pub use presentational::FrameMargins;
pub(crate) use presentational::{
    is_presentational_hint, reaches_descendants as presentational_reaches_descendants,
};
pub(crate) use registered::{
    compute_alone as compute_registered_alone, number as registered_number,
};
use revert::{Mark, Rollbacks};
pub(crate) use revert::{Revert, revert_keyword};
pub(crate) use svg::is_svg_hint;

/// The built-in default stylesheet (M5.6), compiled in — loading it is
/// not resource fetching (no I/O at runtime), so it doesn't bend the
/// "parsers never fetch" rule.
const UA_STYLESHEET_SRC: &str = include_str!("../assets/ua.css");

/// Every element's [`ComputedStyle`] in one document, by [`NodeId`],
/// with those of its pseudo-elements that some rule styles (M16), and
/// the document node's, whose initial values the top-level elements
/// inherit from.
#[derive(Debug, Clone, Default)]
pub struct Styles {
    /// By slot index: the node the styles were computed for, so that a
    /// later node in the same slot is not taken for it, and its styles.
    entries: Vec<Option<(NodeId, Computed)>>,
    /// The font size `rem` resolved against: the root element's, the
    /// first element under the document node.
    rem: f32,
    /// Whether any declaration used a viewport unit (`vw`, `vh`, `vmin`,
    /// `vmax`), so that these styles depend on the viewport the cascade
    /// was given and must be recomputed when it changes (M10.1). A
    /// restyle can set it and never clears it: a stale `true` costs a
    /// needless cascade on resize, a stale `false` would be wrong.
    pub uses_viewport_units: bool,
    /// Whether any declaration used a unit a font's figures give (`ex`,
    /// `ch`, `cap`, `ic`, `lh` and their root forms), so that these
    /// styles are computed again when a web font arrives (G12). Stale
    /// as `uses_viewport_units` is.
    pub uses_font_units: bool,
    /// Whether any element's style made it a size or a scroll-state
    /// container (D72), so that a layout of these styles measures them
    /// (G247): a page with none measures nothing. Stale as
    /// `uses_viewport_units` is, set and never cleared.
    pub has_containers: bool,
    /// Whether any element's style, or a pseudo-element's, snaps its
    /// scrolling or starts it at a target (Scroll Snap 1 and 2), so that
    /// a frame of these styles looks for snap containers (G247): a page
    /// with none looks for none. Set and never cleared, as
    /// `has_containers` is.
    pub has_snaps: bool,
    /// The root element's font, which the root units measure; `None`
    /// for the initial one.
    root_font: Option<Arc<FontProps>>,
    /// The answer each `@media` list gave, the UA sheet's then the
    /// author sheets', which decided the rules in force (M20).
    media: Vec<bool>,
    /// Whether the document was cascaded with scripts running (M24):
    /// then a `noscript` element, whose content is text, makes no box,
    /// its computed `display` left as it is, as the reference browser
    /// leaves it (M25).
    pub scripting: bool,
    /// The document's `color-scheme` meta element's value the styles
    /// were cascaded with (gap batch E, D49).
    page_scheme: Option<Arc<String>>,
    /// The user's colour scheme the styles were cascaded with, where
    /// some element's scheme was chosen by it: what a change of the
    /// preference restyles (D49). `None` where none was.
    preferred_dark: Option<bool>,
    /// The size containers' content boxes the styles were computed with,
    /// as the last layout laid them out (D72).
    container_sizes: ContainerSizes,
    /// The containers a size query or a container unit asked about, each
    /// with the content box it was given: what a layout's sizes are
    /// compared with (D72).
    queried: Queried,
}

impl Styles {
    /// The computed style of `node`: an element's, or the document
    /// node's initial values; `None` for a text node, a node not styled
    /// (detached), or a stale id.
    pub fn get(&self, node: NodeId) -> Option<&ComputedStyle> {
        self.computed(node).map(|computed| &*computed.style)
    }

    /// [`Styles::get`], the style shared (M35.9): what a box and its
    /// fragments hold rather than a copy of it.
    /// The bytes the styles hold (M35.5): an entry an element, and each
    /// distinct style once, however many elements share it.
    pub fn heap_bytes(&self) -> usize {
        let mut seen: std::collections::HashSet<*const ComputedStyle> =
            std::collections::HashSet::new();
        let mut styles = 0;
        let mut count = |style: &Arc<ComputedStyle>| {
            if seen.insert(Arc::as_ptr(style)) {
                styles += std::mem::size_of::<ComputedStyle>();
            }
        };
        let mut pseudo = 0;
        for (_, computed) in self.entries.iter().flatten() {
            count(&computed.style);
            if let Some(styles) = &computed.pseudo {
                pseudo += std::mem::size_of::<PseudoStyles>();
                for style in [
                    &styles.before,
                    &styles.after,
                    &styles.first_letter,
                    &styles.first_line,
                    &styles.marker,
                ]
                .into_iter()
                .flatten()
                {
                    count(style);
                }
            }
        }
        self.entries.capacity() * std::mem::size_of::<Option<(NodeId, Computed)>>()
            + styles
            + pseudo
    }

    pub fn shared(&self, node: NodeId) -> Option<&Arc<ComputedStyle>> {
        self.computed(node).map(|computed| &computed.style)
    }

    /// [`Styles::pseudo`], the style shared (M35.9).
    pub fn pseudo_shared(
        &self,
        node: NodeId,
        pseudo: PseudoElement,
    ) -> Option<&Arc<ComputedStyle>> {
        self.computed(node)?.pseudo.as_deref()?.shared(pseudo)
    }

    /// The computed style of the element `node`'s pseudo-element
    /// `pseudo`, when some rule styles it: `::before`, `::after`,
    /// `::first-letter` and `::first-line` are kept; the others are
    /// matched and set aside, nothing drawing them yet.
    pub fn pseudo(&self, node: NodeId, pseudo: PseudoElement) -> Option<&ComputedStyle> {
        self.computed(node)?.pseudo.as_deref()?.get(pseudo)
    }

    /// The styles of the element `node`'s pseudo-elements that some rule
    /// styles; `None` when none is.
    pub fn pseudo_styles(&self, node: NodeId) -> Option<&PseudoStyles> {
        self.computed(node)?.pseudo.as_deref()
    }

    fn computed(&self, node: NodeId) -> Option<&Computed> {
        match self.entries.get(node.index())? {
            Some((id, computed)) if *id == node => Some(computed),
            _ => None,
        }
    }

    fn set(&mut self, node: NodeId, computed: Computed) {
        dom::hook!("style::set");
        let container = computed.style.container_type;
        self.has_containers |= container.is_size() || container.scroll_state;
        if !self.has_snaps {
            let snaps = |style: &ComputedStyle| {
                let effects = style.effects();
                effects.scroll_snap_type.axis != SnapAxis::None || effects.scroll_initial_target
            };
            self.has_snaps = snaps(&computed.style)
                || computed
                    .pseudo
                    .as_deref()
                    .is_some_and(|pseudo| pseudo.any(snaps));
        }
        let index = node.index();
        if self.entries.len() <= index {
            self.entries.resize(index + 1, None);
        }
        self.entries[index] = Some((node, computed));
    }

    /// The size containers whose content boxes these styles depend on,
    /// a size query or a container unit having asked about them (D72):
    /// what a layout measures again.
    pub fn queried_containers(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.queried.keys().copied()
    }

    /// The containers of `sizes`, a layout's, whose content box is not
    /// the one these styles were computed with (D72).
    pub fn containers_changed(&self, sizes: &ContainerSizes) -> Vec<NodeId> {
        let mut changed: Vec<NodeId> = self
            .queried
            .iter()
            .filter(|&(&node, &used)| (sizes.get(node), sizes.scroll(node)) != used)
            .map(|(&node, _)| node)
            .collect();
        changed.sort_unstable();
        changed
    }

    /// Forgets `node`'s style, if it has one.
    fn forget(&mut self, node: NodeId) {
        if let Some(entry) = self.entries.get_mut(node.index())
            && entry.as_ref().is_some_and(|(id, _)| *id == node)
        {
            *entry = None;
        }
    }
}

/// An element's computed style and those of its pseudo-elements, each
/// shared with the elements made of the same and with the boxes and
/// fragments made of it (M35.9).
#[derive(Debug, Clone, PartialEq, Default)]
struct Computed {
    style: Arc<ComputedStyle>,
    /// `None` when no rule styles any of them, as for most elements.
    pseudo: Option<Box<PseudoStyles>>,
    /// Whether the element's style without its animations, or an
    /// ancestor's, has `display: none` (M22.2): such an element runs no
    /// animation, nor do its pseudo-elements, and its children are
    /// restyled when this changes.
    display_none: bool,
    /// Whether the element is not rendered: its style, its transitions'
    /// values with it, or an ancestor's has `display: none` (M22.1). A
    /// change of style starts no transition for an element that was not
    /// rendered before it.
    unrendered: bool,
    /// The element's style without its transitions' values, when it
    /// differs from [`Computed::style`]: its after-change style
    /// (Transitions 1 §3), which its children's inherit from.
    after_change: Option<Arc<ComputedStyle>>,
    /// Whether one of the element's own declarations inherits a property
    /// no child otherwise inherits (`border-color: inherit`): it reads
    /// more of its parent than what it inherits, and is restyled for any
    /// change of its parent's (G254).
    reads_parent: bool,
}

/// What an element's style inherits from, and what is known of its
/// parent beside: the parent's [`Computed`].
#[derive(Clone, Copy)]
struct Parent<'p> {
    style: &'p ComputedStyle,
    /// The parent's after-change style, when it differs from its style.
    after_change: Option<&'p ComputedStyle>,
    display_none: bool,
    unrendered: bool,
}

impl<'p> Parent<'p> {
    fn of(computed: &'p Computed) -> Self {
        Parent {
            style: &computed.style,
            after_change: computed.after_change.as_deref(),
            display_none: computed.display_none,
            unrendered: computed.unrendered,
        }
    }
}

/// What a change of style knows of a target's style before it, the
/// before-change style (Transitions 1 §3, M22.1).
#[derive(Clone, Copy)]
enum Before<'o> {
    /// Nothing: the cascade starts and cancels no transition, applying
    /// those running as they stand — a first cascade's, a print's, a
    /// recascade's.
    Unknown,
    /// The style the last change left, `None` when there was none: the
    /// target was not rendered, or not in the document.
    Known(Option<&'o ComputedStyle>),
}

/// What a target's animations give its style at the timeline's time.
struct Animating<'a> {
    /// Those in effect, to apply.
    animated: Option<Animated<'a>>,
    /// What those current or in effect animate, which the engine acts as
    /// if `will-change` named (Web Animations §5.6).
    acts: WillChange,
    /// The longhands those in effect animate now: no transition's value
    /// is applied to them, and none starts for them.
    now: Vec<Longhand>,
    /// The longhands they animated in the target's last style, which no
    /// transition starts for either, as in the reference browser.
    before: Vec<Longhand>,
}

/// The styles of an element's pseudo-elements that some rule styles
/// (M16), each cascaded from its own rules with the element's style as
/// the parent it inherits from (Pseudo-Elements 4, tree-abiding
/// pseudo-elements; `::first-letter` and `::first-line` inherit
/// differently when laid out).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PseudoStyles {
    /// `::before`.
    pub before: Option<Arc<ComputedStyle>>,
    /// `::after`.
    pub after: Option<Arc<ComputedStyle>>,
    /// `::first-letter`.
    pub first_letter: Option<Arc<ComputedStyle>>,
    /// `::first-line`.
    pub first_line: Option<Arc<ComputedStyle>>,
    /// `::marker` (M21.6): a list item's alone, whether or not a rule
    /// styles it, none the user agent's.
    pub marker: Option<Arc<ComputedStyle>>,
    /// `::backdrop` (gap batch H): an element's in the top layer alone.
    pub backdrop: Option<Arc<ComputedStyle>>,
    /// `::details-content` (CSS Pseudo-Elements 4 section 5.1): a
    /// `details` element's alone, which the user agent's sheet styles;
    /// and its `::before` and `::after`, which inherit from it.
    pub details_content: Option<Arc<ComputedStyle>>,
    pub details_content_before: Option<Arc<ComputedStyle>>,
    pub details_content_after: Option<Arc<ComputedStyle>>,
}

impl PseudoStyles {
    /// The style of `pseudo`, if it is one kept and some rule styles it.
    pub fn get(&self, pseudo: PseudoElement) -> Option<&ComputedStyle> {
        self.shared(pseudo).map(|style| &**style)
    }

    /// [`PseudoStyles::get`], the style shared (M35.9).
    pub fn shared(&self, pseudo: PseudoElement) -> Option<&Arc<ComputedStyle>> {
        match pseudo {
            PseudoElement::Before => self.before.as_ref(),
            PseudoElement::After => self.after.as_ref(),
            PseudoElement::FirstLetter => self.first_letter.as_ref(),
            PseudoElement::FirstLine => self.first_line.as_ref(),
            PseudoElement::Marker => self.marker.as_ref(),
            PseudoElement::Backdrop => self.backdrop.as_ref(),
            PseudoElement::DetailsContent => self.details_content.as_ref(),
            PseudoElement::DetailsContentBefore => self.details_content_before.as_ref(),
            PseudoElement::DetailsContentAfter => self.details_content_after.as_ref(),
            PseudoElement::Placeholder
            | PseudoElement::Selection
            | PseudoElement::TextHighlight
            | PseudoElement::FileSelectorButton
            | PseudoElement::Picker
            | PseudoElement::PickerIcon
            | PseudoElement::Checkmark
            | PseudoElement::WebkitPrefixed
            | PseudoElement::Slotted
            | PseudoElement::Part
            | PseudoElement::Highlight => None,
        }
    }

    /// Whether any of the styles kept is one `test` holds of.
    fn any(&self, test: impl Fn(&ComputedStyle) -> bool) -> bool {
        [
            &self.before,
            &self.after,
            &self.first_letter,
            &self.first_line,
            &self.marker,
            &self.backdrop,
            &self.details_content,
            &self.details_content_before,
            &self.details_content_after,
        ]
        .into_iter()
        .flatten()
        .any(|style| test(style))
    }
}

/// What a style is computed for: an element, or one of the
/// pseudo-elements whose styles are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Element,
    Before,
    After,
    FirstLetter,
    FirstLine,
    Marker,
    Backdrop,
    DetailsContent,
    DetailsContentBefore,
    DetailsContentAfter,
    /// A custom highlight (gap batch H), whose style is asked for alone
    /// ([`Recascader::highlight`]), never kept with the element's: not
    /// among [`Target::ALL`], its declarations matched into the
    /// element's place.
    Highlight,
}

impl Target {
    /// The targets in the order [`Matched`] keeps them.
    const ALL: [Target; 10] = [
        Target::Element,
        Target::Before,
        Target::After,
        Target::FirstLetter,
        Target::FirstLine,
        Target::Marker,
        Target::Backdrop,
        Target::DetailsContent,
        Target::DetailsContentBefore,
        Target::DetailsContentAfter,
    ];

    /// Whether `content`'s `normal` computes to `none`: on `::before`
    /// and `::after` (CSS 2.2 section 12.2), `::details-content`'s among them;
    /// on `::marker` it stays.
    fn takes_content(self) -> bool {
        matches!(
            self,
            Target::Before
                | Target::After
                | Target::DetailsContentBefore
                | Target::DetailsContentAfter
        )
    }

    /// The pseudo-element it is, when its animations run (M22.2): the
    /// reference browser animates `::before`, `::after`, `::marker` and
    /// `::backdrop`, and not `::first-letter`, `::first-line` or a
    /// highlight.
    fn animated(self) -> Option<PseudoElement> {
        match self {
            Target::Before => Some(PseudoElement::Before),
            Target::After => Some(PseudoElement::After),
            Target::Marker => Some(PseudoElement::Marker),
            Target::Backdrop => Some(PseudoElement::Backdrop),
            Target::Element
            | Target::FirstLetter
            | Target::FirstLine
            | Target::DetailsContent
            | Target::DetailsContentBefore
            | Target::DetailsContentAfter
            | Target::Highlight => None,
        }
    }

    /// The target a pseudo-element's style is kept as; `None` for the
    /// ones matched and set aside, which nothing draws yet.
    fn of(pseudo: PseudoElement) -> Option<Target> {
        match pseudo {
            PseudoElement::Before => Some(Target::Before),
            PseudoElement::After => Some(Target::After),
            PseudoElement::FirstLetter => Some(Target::FirstLetter),
            PseudoElement::FirstLine => Some(Target::FirstLine),
            PseudoElement::Marker => Some(Target::Marker),
            PseudoElement::Backdrop => Some(Target::Backdrop),
            PseudoElement::DetailsContent => Some(Target::DetailsContent),
            PseudoElement::DetailsContentBefore => Some(Target::DetailsContentBefore),
            PseudoElement::DetailsContentAfter => Some(Target::DetailsContentAfter),
            PseudoElement::Placeholder
            | PseudoElement::Selection
            | PseudoElement::TextHighlight
            | PseudoElement::FileSelectorButton
            | PseudoElement::Picker
            | PseudoElement::PickerIcon
            | PseudoElement::Checkmark
            | PseudoElement::WebkitPrefixed
            | PseudoElement::Slotted
            | PseudoElement::Part
            | PseudoElement::Highlight => None,
        }
    }

    /// Whether a declaration of the property `name` applies to the
    /// target (M16). `content` applies to `::before` and `::after` only.
    /// `::first-line` takes the properties Pseudo-Elements 4 §2.1.2
    /// lists that this engine has — the font's, colour, background,
    /// `text-transform`, letter and word spacing and `vertical-align` —
    /// and `::first-letter` those of §2.2.4: these, margins, padding and
    /// borders, and `float`, which every browser honours for a drop cap.
    /// `::marker` takes those Lists 3 §3.1.1 lets apply that this engine
    /// has (M21.6): the inherited properties of text — the font's,
    /// colour, `text-transform`, `white-space`, letter and word spacing
    /// and `quotes`, which its quote marks read — and `direction` and
    /// `content`. The element, `::before` and `::after` take every other
    /// property. Each takes the custom properties, which the cascade
    /// computes before these.
    fn applies(self, name: &str) -> bool {
        let line = matches!(
            name,
            "font-size"
                | "font-family"
                | "font"
                | "font-weight"
                | "font-style"
                | "font-stretch"
                | "font-variant"
                | "font-variant-caps"
                | "font-variant-ligatures"
                | "font-variant-alternates"
                | "font-variant-numeric"
                | "font-variant-east-asian"
                | "font-variant-position"
                | "font-variant-emoji"
                | "font-feature-settings"
                | "font-kerning"
                | "line-height"
                | "color"
                | "background-color"
                | "background"
                | "background-image"
                | "background-repeat"
                | "background-position"
                | "background-position-x"
                | "background-position-y"
                | "background-size"
                | "background-origin"
                | "background-clip"
                | "background-attachment"
                | "text-transform"
                | "letter-spacing"
                | "word-spacing"
                | "vertical-align"
                | "text-shadow"
        );
        match self {
            // A highlight takes the properties CSS Pseudo 4 section 3.2
            // lets apply that this engine has: its colours, its
            // decoration's and its shadow.
            Target::Highlight => matches!(
                name,
                "color"
                    | "background-color"
                    | "text-decoration"
                    | "text-decoration-line"
                    | "text-decoration-style"
                    | "text-decoration-color"
                    | "text-decoration-thickness"
                    | "text-underline-offset"
                    | "text-underline-position"
                    | "text-shadow"
            ),
            Target::FirstLine => line,
            Target::FirstLetter => {
                line || name == "float"
                    || (["margin", "padding", "border"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                        // A table's, not the box's border.
                        && !matches!(name, "border-collapse" | "border-spacing"))
            }
            Target::Marker => {
                matches!(
                    name,
                    "font-size"
                        | "font-family"
                        | "font"
                        | "font-weight"
                        | "font-style"
                        | "font-stretch"
                        | "font-variant"
                        | "font-variant-caps"
                        | "font-variant-ligatures"
                        | "font-variant-alternates"
                        | "font-variant-numeric"
                        | "font-variant-east-asian"
                        | "font-variant-position"
                        | "font-variant-emoji"
                        | "font-feature-settings"
                        | "font-kerning"
                        | "color"
                        | "text-transform"
                        | "white-space"
                        | "white-space-collapse"
                        | "text-wrap-mode"
                        | "text-shadow"
                        | "letter-spacing"
                        | "word-spacing"
                        | "quotes"
                        | "direction"
                        | "unicode-bidi"
                        | "content"
                        | "line-height"
                        | "hyphens"
                        | "line-break"
                        | "overflow-wrap"
                        | "tab-size"
                        | "word-break"
                        | "cursor"
                ) || motion::is_motion_property(name)
                    || (name.starts_with("animation") && timelines::is_timeline_property(name))
            }
            // An element takes `content` too (M23.3): an image alone
            // replaces its content, which the box tree honours, and the
            // rest it ignores.
            Target::Element | Target::DetailsContent => true,
            Target::Before
            | Target::After
            | Target::Backdrop
            | Target::DetailsContentBefore
            | Target::DetailsContentAfter => true,
        }
    }
}

/// What a font can say about itself, for the units that need it
/// (Values 4 section 6.1.1). `style` has no font of its own (the
/// layering contract: `text` is a sibling, not a dependency), so the
/// caller lends one through this trait; without one, the units take
/// [`FontUnits::fallback`].
pub trait FontMeasure {
    /// The figures of the first available font of `font` at
    /// `font_size_px`, in CSS pixels.
    fn units(&self, font: &FontProps, font_size_px: f32) -> FontUnits;
}

/// A font's figures for the font-relative units, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontUnits {
    /// `ex`: the x-height.
    pub x_height: f32,
    /// `ch`: the advance of the digit zero.
    pub zero_advance: f32,
    /// `cap`: the cap height.
    pub cap_height: f32,
    /// `ic`: the advance of the water ideograph, U+6C34.
    pub ic_advance: f32,
    /// A `normal` line's height, which `lh` is then.
    pub normal_line_height: f32,
}

impl FontUnits {
    /// The figures Values 4 allows when the font gives none: half an em
    /// for `ex` and `ch`, an em for `ic`, the em's ascent-like 0.7 for
    /// `cap`, and 1.2 ems for a normal line.
    pub fn fallback(font_size_px: f32) -> FontUnits {
        FontUnits {
            x_height: font_size_px * 0.5,
            zero_advance: font_size_px * 0.5,
            cap_height: font_size_px * 0.7,
            ic_advance: font_size_px,
            normal_line_height: font_size_px * 1.2,
        }
    }
}

/// Whether an element's `style` attribute styles it (M33.1): a document's
/// Content Security Policy may refuse it.
pub trait InlineStyles {
    /// Whether `element`'s `style` attribute, whose value is `value`,
    /// applies.
    fn allowed(&self, element: NodeId, value: &str) -> bool;
}

/// The top layer's say in its elements' `overlay` (CSS Position 4
/// sections 3.2 and 3.3, D76): an important declaration of the user
/// agent's, `auto` for an element in the top layer and `none` for one
/// whose removal from it is pending. A transition of `overlay` with
/// `allow-discrete` keeps the computed value `auto` until it ends, as
/// `display`'s keeps a box (Transitions 2 section 3.2), and the element
/// pending removal leaves the top layer once its computed `overlay` is
/// `none`: the removal waits for the transition.
pub trait TopLayer {
    /// The `overlay` the top layer declares for `element`, `None` for
    /// one it neither holds nor is removing.
    fn overlay(&self, element: NodeId) -> Option<Overlay>;
}

/// What the cascade resolves relative units against (M10.1), and what
/// it knows of the document from outside it (M16).
#[derive(Clone, Copy, Default)]
pub struct CascadeContext<'a> {
    /// The viewport's size in CSS pixels, for `vw`, `vh`, `vmin` and
    /// `vmax`. Zero when unknown: those units then resolve to zero.
    pub viewport_width: f32,
    pub viewport_height: f32,
    /// The font measures for `ex` and `ch`, or `None` for the fallbacks.
    pub fonts: Option<&'a dyn FontMeasure>,
    /// The language the protocol gave the document (HTTP's
    /// `Content-Language`, when it names one), `:lang()`'s last resort
    /// for an element no `lang` attribute or `meta` pragma gives one.
    pub language: Option<&'a str>,
    /// What `@media` lists are evaluated against (M20): a screen of no
    /// size by default. Its size is the caller's to keep with the
    /// viewport's above.
    pub media: Media,
    /// The animations the document runs and its timeline (M22.2), which
    /// the cascade keeps as it computes styles: an element's animations
    /// are matched to its `animation-name`, started and cancelled, and
    /// their values at the timeline's time applied. `None` runs none,
    /// the cascade of a test or a fuzz target. A [`Recascader`] reads
    /// them and changes nothing.
    pub animations: Option<&'a RefCell<Animations>>,
    /// What decides whether an element's `style` attribute applies
    /// (M33.1); `None` applies every one.
    pub inline_styles: Option<&'a dyn InlineStyles>,
    /// The margins of the iframe the document is in, which its `body`
    /// takes.
    pub frame_margins: FrameMargins,
    /// What the top layer declares of its elements' `overlay`; `None`
    /// when no element is in it.
    pub top_layer: Option<&'a dyn TopLayer>,
}

/// Resolves the computed style of every element under `document`'s
/// root, cascading the built-in UA stylesheet (lowest precedence) under
/// `author_sheets` (in document order) and each element's inline
/// `style` attribute (highest precedence short of `!important` — see
/// `tier`), with the viewport and font units resolved against
/// `context`. `CascadeContext::default()`, no viewport and no font, is
/// what a test or a fuzz target wants. An element outside the tree, a
/// detached one, is not styled. A caller that cascades the same sheets
/// again, or restyles, keeps them as [`Stylesheets`] instead.
pub fn cascade_document(
    document: &Document,
    author_sheets: &[css::Stylesheet],
    context: &CascadeContext<'_>,
) -> Styles {
    Stylesheets::new(author_sheets.to_vec()).cascade(document, context)
}

/// A shadow tree's sheets as [`Stylesheets::with_shadow_trees`] takes
/// them (D69): its root, its sheets in order, and each sheet's implicit
/// scoping root.
pub type ShadowTreeSheets = (NodeId, Vec<css::Stylesheet>, Vec<Option<NodeId>>);

/// A document's author stylesheets prepared for the cascade (M13.1):
/// each style rule's selectors parsed once, with the conditions and the
/// layer it is in (M20), and what the selectors depend on, the built-in
/// UA sheet's included, which says what a change to the document can
/// restyle. Made when the stylesheets change and kept for every cascade
/// and restyle until they do again.
#[derive(Debug)]
pub struct Stylesheets {
    sheets: Vec<css::Stylesheet>,
    rules: RuleSet,
    features: SelectorFeatures,
    /// The custom properties the document's scripts registered
    /// (`CSS.registerProperty`, D73), in order.
    registered: Vec<PropertyRegistration>,
    /// This set's own number, never another's in the process, a copy's
    /// included: what says the declarations a custom property map was
    /// computed from were these sheets' (M21.1, `crate::custom`).
    id: u64,
    /// Each shadow tree's rules (gap batch H, D69), by its shadow root:
    /// its sheets made a set once, shared by every tree of the same
    /// sheets, a component of a thousand instances having one.
    shadow: HashMap<NodeId, Arc<Stylesheets>>,
}

impl Clone for Stylesheets {
    /// A copy, with a number of its own: its declarations are elsewhere
    /// in memory.
    fn clone(&self) -> Self {
        Stylesheets {
            sheets: self.sheets.clone(),
            rules: self.rules.clone(),
            features: self.features.clone(),
            registered: self.registered.clone(),
            id: next_sheets_id(),
            shadow: self.shadow.clone(),
        }
    }
}

/// A number for a set of stylesheets, never given before in the process.
fn next_sheets_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Default for Stylesheets {
    fn default() -> Self {
        Stylesheets::new(Vec::new())
    }
}

impl Stylesheets {
    /// `sheets`, in document order, prepared.
    pub fn new(sheets: Vec<css::Stylesheet>) -> Self {
        let mut features = ua().features.clone();
        let rules = RuleSet::new(&sheets, &mut features);
        Stylesheets {
            sheets,
            rules,
            features,
            registered: Vec::new(),
            id: next_sheets_id(),
            shadow: HashMap::new(),
        }
    }

    /// These sheets with the custom properties a document's scripts
    /// registered, `registered`, in order (D73): each a registration of
    /// its name that wins over every `@property` rule's.
    pub fn with_registered(mut self, registered: Vec<PropertyRegistration>) -> Self {
        self.registered = registered;
        self
    }

    /// The custom properties the document's scripts registered, as
    /// [`Stylesheets::with_registered`] was given them.
    pub fn registered(&self) -> &[PropertyRegistration] {
        &self.registered
    }

    /// `sheets`, in document order, prepared, each with the implicit
    /// scoping root `roots` gives at its index, if any: what an `@scope`
    /// rule of it without roots of its own scopes to, the parent element
    /// of the node that owns the sheet (Cascade 6 section 2.5.1). One in
    /// a sheet with none scopes to the root element.
    pub fn with_scope_roots(sheets: Vec<css::Stylesheet>, roots: Vec<Option<NodeId>>) -> Self {
        let mut prepared = Stylesheets::new(sheets);
        prepared.rules.set_scope_roots(roots);
        prepared
    }

    /// Each sheet's implicit scoping root, by its index, as
    /// [`Stylesheets::with_scope_roots`] was given them.
    pub fn scope_roots(&self) -> &[Option<NodeId>] {
        self.rules.scope_roots()
    }

    /// These sheets with the shadow trees' (D69), `trees` each a shadow
    /// root, its sheets in order, its `<style>` and `<link>` elements'
    /// then its adopted ones, and each sheet's implicit scoping root as
    /// [`Stylesheets::with_scope_roots`] takes them, the host for none:
    /// a tree's rules match its own elements, and
    /// its host, `::slotted()` and `::part()` as CSS Scoping 1 says. The
    /// trees of the same sheets share one set, and what the sets'
    /// selectors depend on, the document's changes reach as these
    /// sheets' own.
    pub fn with_shadow_trees(mut self, trees: Vec<ShadowTreeSheets>) -> Self {
        let mut made: Vec<Arc<Stylesheets>> = Vec::new();
        for (root, sheets, roots) in trees {
            if sheets.is_empty() {
                continue;
            }
            let same = |set: &&Arc<Stylesheets>| set.sheets == sheets && set.scope_roots() == roots;
            let set = match made.iter().find(same) {
                Some(set) => set.clone(),
                None => {
                    let set = Arc::new(Stylesheets::with_scope_roots(sheets, roots));
                    self.features.absorb(&set.features);
                    made.push(set.clone());
                    set
                }
            };
            self.shadow.insert(root, set);
        }
        self
    }

    /// Whether the shadow trees' sheets are `trees`, as
    /// [`Stylesheets::with_shadow_trees`] takes them.
    pub fn same_shadow_trees(&self, trees: &[ShadowTreeSheets]) -> bool {
        let given: Vec<&ShadowTreeSheets> = trees
            .iter()
            .filter(|(_, sheets, _)| !sheets.is_empty())
            .collect();
        given.len() == self.shadow.len()
            && given.iter().all(|(root, sheets, roots)| {
                self.shadow.get(root).is_some_and(|set| {
                    set.sheets == *sheets && set.scope_roots() == roots.as_slice()
                })
            })
    }

    /// The sheets, in document order.
    pub fn sheets(&self) -> &[css::Stylesheet] {
        &self.sheets
    }

    /// What the selectors, the UA sheet's among them, depend on.
    pub fn features(&self) -> &SelectorFeatures {
        &self.features
    }

    /// [`cascade_document`] with these sheets.
    pub fn cascade(&self, document: &Document, context: &CascadeContext<'_>) -> Styles {
        self.cascader(document, context).cascade(None)
    }

    /// [`Stylesheets::cascade`] as a change of style from `before`, the
    /// styles the new ones replace (M22.1): each element's transitions
    /// started, replaced and cancelled as a restyle's are, so that a
    /// change of the sheets, of the viewport or of the medium starts
    /// transitions as a change of the document does.
    pub fn cascade_from(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        before: &Styles,
    ) -> Styles {
        self.cascader(document, context).cascade(Some(before))
    }

    /// Whether the rules in force under `media` differ from those
    /// `styles` were cascaded with: whether any `@media` list, the UA
    /// sheet's or these sheets', answers otherwise (M20). What a caller
    /// asks before reusing styles in a new environment, a resize or a
    /// change of the user's preferences; a few comparisons a list.
    pub fn media_changed(&self, styles: &Styles, media: &Media) -> bool {
        self.answers(media) != styles.media
            || styles
                .preferred_dark
                .is_some_and(|dark| dark != (media.preferences.color_scheme == ColorScheme::Dark))
    }

    /// What the `@page` rules in force under `media` say of a page at
    /// `position` (M20): their `size` and margins, cascaded as a style
    /// rule's declarations are, by origin and importance, then layer,
    /// then the specificity of the most specific of a rule's page
    /// selectors that matches, then order. `em` and `rem` in them are of
    /// `rem`, the root element's font size, from which the page context
    /// inherits (Paged Media 3 §6). A CSS-wide keyword leaves the
    /// descriptor to the print's own.
    pub fn page_style(&self, media: &Media, rem: f32, position: PagePosition) -> PageStyle {
        let ua = ua();
        let ua_pages = ua
            .rules
            .pages_in_force(std::slice::from_ref(&ua.sheet), &ua.rules.answers(media));
        let author_pages = self
            .rules
            .pages_in_force(&self.sheets, &self.rules.answers(media));
        let mut applied = Vec::new();
        let mut order = 0u32;
        for (base, pages) in [
            (TierBase::UserAgent, &ua_pages),
            (TierBase::AuthorSheet, &author_pages),
        ] {
            for page in pages {
                let specificity = page
                    .selectors
                    .iter()
                    .filter(|selector| selector.matches(position))
                    .map(|selector| selector.specificity())
                    .max();
                let Some(specificity) = specificity else {
                    continue;
                };
                for declaration in page.declarations {
                    let layer = if declaration.important {
                        page.important_layer
                    } else {
                        page.layer
                    };
                    applied.push((
                        tier(base, declaration.important),
                        layer,
                        specificity,
                        order,
                        declaration,
                    ));
                    order += 1;
                }
            }
        }
        applied
            .sort_by_key(|&(tier, layer, specificity, order, _)| (tier, layer, specificity, order));
        let context = CascadeContext {
            viewport_width: media.width,
            viewport_height: media.height,
            media: *media,
            ..CascadeContext::default()
        };
        let viewport_used = Cell::new(false);
        let substitutions = Substitutions::new(self.id);
        let resolution = Resolution {
            rem,
            root: false,
            page_scheme: None,
            context: &context,
            viewport_used: &viewport_used,
            fonts_used: None,
            root_font: None,
            root_size: rem,
            scheme_read: None,
            attributes: &[],
            substitutions: &substitutions,
            containers: None,
            units_from: None,
            containers_used: &viewport_used,
        };
        let bases = LengthBases {
            em: rem,
            font: crate::computed::initial_font(),
            line: (crate::computed::initial_font(), rem),
            resolution: &resolution,
            color: Color::BLACK,
            dark: false,
        };
        let mut style = PageStyle::default();
        for (.., declaration) in applied {
            let value = declaration.value.as_slice();
            let wide = css_wide_keyword(value).is_some();
            match declaration.name.as_str() {
                "size" if wide => style.size = PageStyle::default().size,
                "size" => {
                    if let Some(size) = page::parse_size(value, &bases) {
                        style.size = size;
                    }
                }
                "margin" if wide => style.margin = Default::default(),
                "margin" => {
                    if let Some(margins) = page::parse_page_margin(value, &bases) {
                        style.margin = margins.map(Some);
                    }
                }
                side @ ("margin-top" | "margin-right" | "margin-bottom" | "margin-left") => {
                    let index = edge_index(side.trim_start_matches("margin-"));
                    if wide {
                        style.margin[index] = None;
                    } else if let Some(margin) = page::parse_page_margin_side(value, &bases) {
                        style.margin[index] = Some(margin);
                    }
                }
                _ => {}
            }
        }
        style
    }

    /// The faces of the `@font-face` rules in force under `media`, in
    /// order (D54).
    pub fn font_faces(&self, media: &Media) -> Vec<crate::font_face::FontFaceRule> {
        self.rules
            .font_faces_in_force(&self.rules.answers(media))
            .into_iter()
            .cloned()
            .collect()
    }

    /// The keyframes of the `@keyframes` rule named `name` in force under
    /// `media`, processed, as a CSS animation's effect reads them (D75);
    /// `None` when no rule of the name is. The name is looked up as the
    /// cascade looks it up for the tree whose rule gave it, `tree` (a
    /// shadow root, D69): in that tree's sheets, then each host's tree
    /// outwards, then the document's (CSS Scoping 1 section 3.5).
    pub fn keyframes_named(
        &self,
        document: &Document,
        tree: Option<NodeId>,
        name: &str,
        media: &Media,
    ) -> Option<Vec<crate::keyframes::CssKeyframe>> {
        dom::hook!("style::keyframes_named");
        let processed = |rule: &KeyframesInForce<'_>| {
            rule.keyframes
                .keyframes
                .iter()
                .map(|keyframe| crate::keyframes::CssKeyframe {
                    offset: keyframe.offset,
                    easing: keyframe.easing.clone(),
                    composite: keyframe.composite,
                    declarations: rule.declarations(keyframe).cloned().collect(),
                })
                .collect()
        };
        let mut at = tree;
        let mut steps = 0;
        while let Some(root) = at.filter(|_| steps < dom::MAX_DEPTH) {
            if let Some(set) = self.shadow.get(&root)
                && set.rules.has_keyframes()
            {
                let answers = set.rules.answers(media);
                let in_force = set.rules.keyframes_in_force(&set.sheets, &answers);
                if let Some(rule) = in_force.get(name) {
                    return Some(processed(rule));
                }
            }
            at = document
                .shadow_host(root)
                .map(|host| document.root_of(host))
                .filter(|&outer| document.is_shadow_root(outer));
            steps += 1;
        }
        let answers = self.rules.answers(media);
        let in_force = self.rules.keyframes_in_force(&self.sheets, &answers);
        Some(processed(in_force.get(name)?))
    }

    /// The answers of the UA sheet's `@media` lists, then these sheets',
    /// then the shadow trees', by their roots.
    fn answers(&self, media: &Media) -> Vec<bool> {
        let mut answers = ua().rules.answers(media);
        answers.extend(self.rules.answers(media));
        answers.extend(self.shadow_answers(media));
        answers
    }

    /// The answers of the shadow trees' `@media` lists, by their roots.
    fn shadow_answers(&self, media: &Media) -> Vec<bool> {
        let mut roots: Vec<&NodeId> = self.shadow.keys().collect();
        roots.sort_unstable();
        roots
            .into_iter()
            .flat_map(|root| self.shadow[root].rules.answers(media))
            .collect()
    }

    /// A cascader over `document` with the rules in force under
    /// `context`'s medium, the `@keyframes` rules among them when the
    /// context runs animations (the user agent's sheet has none).
    fn cascader<'a>(
        &'a self,
        document: &'a Document,
        context: &'a CascadeContext<'a>,
    ) -> Cascader<'a> {
        let ua = ua();
        let ua_answers = ua.rules.answers(&context.media);
        let answers = self.rules.answers(&context.media);
        let ua_rules = ua
            .rules
            .in_force(std::slice::from_ref(&ua.sheet), &ua_answers);
        let author = self.rules.in_force(&self.sheets, &answers);
        let keyframes = if context.animations.is_some() && self.rules.has_keyframes() {
            self.rules.keyframes_in_force(&self.sheets, &answers)
        } else {
            HashMap::new()
        };
        // The registrations in force: the `@property` rules', then the
        // scripts', which win (D73).
        let registry = Registry::new(self.rules.properties_in_force(&answers), &self.registered);
        // The rules of an element's starting style, `@starting-style`'s
        // among them, where a sheet has one and transitions run.
        let starting = (context.animations.is_some() && self.rules.has_starting_style())
            .then(|| self.rules.in_force_of(&self.sheets, &answers, true));
        let mut media = ua_answers;
        media.extend(answers);
        media.extend(self.shadow_answers(&context.media));
        let mut cascader = Cascader::new(document, ua_rules, author, context, media, self.id);
        cascader.substitutions = Substitutions::new(self.id).with_registry(Arc::new(registry));
        cascader.keyframes = keyframes;
        cascader.shadow = self
            .shadow
            .iter()
            .map(|(&root, set)| {
                let answers = set.rules.answers(&context.media);
                (root, set.rules.in_force(&set.sheets, &answers))
            })
            .collect();
        // Each shadow tree's `@keyframes` rules, its names its own (CSS
        // Scoping 1 section 3.5).
        if context.animations.is_some() {
            cascader.shadow_keyframes = self
                .shadow
                .iter()
                .filter(|(_, set)| set.rules.has_keyframes())
                .map(|(&root, set)| {
                    let answers = set.rules.answers(&context.media);
                    (root, set.rules.keyframes_in_force(&set.sheets, &answers))
                })
                .collect();
        }
        cascader.starting = starting;
        cascader
    }

    /// Brings `styles`, computed for `document` with these sheets and
    /// `context`, up to date after `changes`, the records of
    /// everything done to the document since in the order it was done
    /// (M13.1). The elements the changes can restyle, by the rules of
    /// [`crate::invalidation`], are restyled shallowest first, each
    /// from its parent's style; one whose style changed has its
    /// children restyled too, since they inherit from it, and one
    /// marked with its subtree has all of it restyled. The styles of
    /// nodes taken out of the tree are forgotten. Where that cannot
    /// give what a full cascade would — the journal overflowed, the
    /// root's font size, which `rem` reads, changed, or an `@media`
    /// list answers otherwise in `context` — the document is cascaded
    /// whole. The result is what a full cascade would give.
    pub fn restyle(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        styles: &mut Styles,
        changes: &[Change],
    ) -> Restyle {
        self.restyle_animated(document, context, styles, changes, &[])
    }

    /// [`Stylesheets::restyle`], with the elements `animated` restyled
    /// too, as elements whose own style changed: those whose animations
    /// give something new at the timeline's time (M22.3). A node taken
    /// out of the document, put back or not, has its animations
    /// cancelled, as in the reference browser, where an element moved
    /// starts its animations again.
    pub fn restyle_animated(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        styles: &mut Styles,
        changes: &[Change],
        animated: &[NodeId],
    ) -> Restyle {
        self.restyle_flushed(document, context, styles, changes, animated, None)
    }

    /// [`Stylesheets::restyle_animated`], an element `styles` has not met
    /// taking its before-change style from `flushed`, the styles a
    /// script's style flush computed since (CSS Transitions 1 section 3,
    /// G46): one a script styled before it changed it transitions from
    /// that style, as the reference browser has it.
    pub fn restyle_flushed(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        styles: &mut Styles,
        changes: &[Change],
        animated: &[NodeId],
        flushed: Option<&Styles>,
    ) -> Restyle {
        dom::hook!("style::restyle_flushed");
        if let Some(animations) = context.animations
            && !animations.borrow().is_empty()
        {
            let taken_out: HashSet<NodeId> = changes
                .iter()
                .filter_map(|change| match change {
                    Change::Removed { child, .. } => Some(*child),
                    _ => None,
                })
                .flat_map(|child| document.subtree(child))
                .collect();
            if !taken_out.is_empty() {
                animations
                    .borrow_mut()
                    .retain(|node| !taken_out.contains(&node));
            }
        }
        self.restyle_reaching(document, context, styles, changes, animated, &[], flushed)
    }

    /// The containers of `sizes`, a layout's, whose content box changed
    /// from the one `styles` were computed with, restyled with their
    /// subtrees against it (D72): what the engine does after a layout,
    /// at most [`crate::container::MAX_CONTAINER_PASSES`] times. `None`
    /// when no container's size changed.
    pub fn restyle_containers(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        styles: &mut Styles,
        sizes: &ContainerSizes,
    ) -> Option<Restyle> {
        dom::hook!("style::restyle_containers");
        let changed = styles.containers_changed(sizes);
        styles.container_sizes = sizes.clone();
        if changed.is_empty() {
            return None;
        }
        Some(self.restyle_reaching(document, context, styles, &[], &[], &changed, None))
    }

    /// [`Stylesheets::restyle_flushed`], with the elements `subtrees`
    /// restyled with everything under them too.
    #[allow(clippy::too_many_arguments)]
    fn restyle_reaching(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        styles: &mut Styles,
        changes: &[Change],
        animated: &[NodeId],
        subtrees: &[NodeId],
        flushed: Option<&Styles>,
    ) -> Restyle {
        dom::hook!("style::restyle_reaching");
        let invalidation = invalidate(document, &self.features, changes);
        let mut cascader = self.cascader(document, context);
        cascader.root_font = styles.root_font.clone();
        cascader.container_sizes = styles.container_sizes.clone();
        let preferred_dark = context.media.preferences.color_scheme == ColorScheme::Dark;
        if invalidation.everything
            || cascader.media != styles.media
            || cascader.page_scheme != styles.page_scheme
            || styles
                .preferred_dark
                .is_some_and(|dark| dark != preferred_dark)
        {
            *styles = cascader.cascade(Some(styles));
            return Restyle::full(styles);
        }
        for removed in &invalidation.removed {
            for node in document.shadow_including_subtree(*removed) {
                styles.forget(node);
            }
        }
        let mut restyle = Restyle::default();
        // Whether a container's type, names or custom properties can
        // change what its subtree's queries or units answer (D72).
        let watches_containers = self.rules.has_containers() || !styles.queried.is_empty();
        // By depth, shallowest first: every ancestor's style is final
        // before a node is restyled from its parent's.
        let mut levels: BTreeMap<usize, Vec<NodeId>> = BTreeMap::new();
        let mut pending = invalidation.dirty;
        let mut depths = invalidation.depths;
        for &node in animated {
            if document.is_element(node) && styles.get(node).is_some() {
                pending.entry(node).or_insert(Reach::Element);
            }
        }
        for &node in subtrees {
            if document.is_element(node) && styles.get(node).is_some() {
                pending.insert(node, Reach::Subtree);
            }
        }
        // Through the flat tree (D68): what an element inherits from.
        for &node in pending.keys() {
            levels
                .entry(depths.depth(document, node).unwrap_or(0))
                .or_default()
                .push(node);
        }
        let first_element = document.document_element();
        let no_parent = Computed::default();
        while let Some((depth, nodes)) = levels.pop_first() {
            for node in nodes {
                let Some(reach) = pending.remove(&node) else {
                    continue; // listed twice
                };
                let parent = document
                    .flat_parent(node)
                    .and_then(|parent| styles.computed(parent))
                    .unwrap_or(&no_parent);
                // The root element resolves `rem` against the initial
                // font size, every other element against the root's.
                let rem = if Some(node) == first_element {
                    ComputedStyle::default().font_size_px
                } else {
                    styles.rem
                };
                // The style it had is its before-change style, if it was
                // rendered (M22.1).
                let old = styles.computed(node);
                // One these styles have not met takes the script's flush
                // of it, its pseudo-elements' styles with it (G46).
                let had = old.or_else(|| flushed.and_then(|f| f.computed(node)));
                let before =
                    Before::Known(had.filter(|had| !had.unrendered).map(|had| &*had.style));
                // A change to a pseudo-element's style counts as one to
                // the element's: how layout hears of it.
                let computed =
                    cascader.style_of(node, Parent::of(parent), rem, before, had, styles);
                restyle.computed += 1;
                let changed = old != Some(&computed);
                // What the queries of the elements under a container read
                // of it, its type, names and custom properties, changed:
                // all of them are restyled (D72).
                let container_changed = watches_containers
                    && changed
                    && old.is_none_or(|old| {
                        old.style.container_type != computed.style.container_type
                            || old.style.container_names != computed.style.container_names
                            || old.style.custom != computed.style.custom
                    });
                // What layout reads, the style and the pseudo-elements',
                // changed; or only what the cascade keeps beside them,
                // which the children read: the after-change style a
                // transition's start or end alone changes (M22.1).
                let shown = changed
                    && old.is_none_or(|old| {
                        old.style != computed.style || old.pseudo != computed.pseudo
                    });
                let composited = old
                    .filter(|_| shown)
                    .and_then(|old| composited_change(node, old, &computed));
                // Whether its children can be styled otherwise for it
                // (G254): a change of what none of them reads, a border's
                // colour at an animation's frame, restyles none but those
                // that read it whole.
                let children_read =
                    changed && old.is_none_or(|old| reaches_children(old, &computed));
                // An equal style leaves the one in place, whose effects
                // the layout's boxes carry: what a later composited
                // change finds them by (M22.4).
                let computed = match old {
                    Some(old) if changed && !shown => Computed {
                        style: old.style.clone(),
                        pseudo: old.pseudo.clone(),
                        ..computed
                    },
                    _ => computed,
                };
                if changed {
                    styles.set(node, computed);
                }
                match composited {
                    Some(composited) => restyle.composited.extend(composited),
                    None if shown => restyle.changed.push(node),
                    None => {}
                }
                if changed || reach == Reach::Subtree {
                    let child_reach = if reach == Reach::Subtree || container_changed {
                        Reach::Subtree
                    } else {
                        Reach::Element
                    };
                    // Each child when they can read the change, else those
                    // that read their parent whole, and any not styled yet.
                    let every = children_read || child_reach == Reach::Subtree;
                    let children: Vec<NodeId> = document
                        .flat_children(node)
                        .filter(|&child| document.is_element(child))
                        .filter(|&child| {
                            every || styles.computed(child).is_none_or(|c| c.reads_parent)
                        })
                        .collect();
                    for child in children {
                        match pending.get_mut(&child) {
                            Some(r) => *r = (*r).max(child_reach),
                            None => {
                                pending.insert(child, child_reach);
                                levels.entry(depth + 1).or_default().push(child);
                            }
                        }
                    }
                }
            }
        }
        styles.uses_viewport_units |= cascader.viewport_used.get();
        styles.uses_font_units |= cascader.fonts_used.get();
        styles.queried.extend(cascader.queried.take());
        if cascader.scheme_read.get() {
            styles.preferred_dark = Some(preferred_dark);
        }
        let rem = first_element
            .and_then(|root| styles.get(root))
            .map_or(ComputedStyle::default().font_size_px, |s| s.font_size_px);
        let root_font = first_element
            .and_then(|root| styles.get(root))
            .and_then(|s| s.font.clone());
        // The root's font matters to the root units alone.
        if rem != styles.rem || (root_font != styles.root_font && styles.uses_font_units) {
            *styles = cascader.cascade(Some(styles));
            return Restyle::full(styles);
        }
        styles.root_font = root_font;
        restyle
    }
}

/// What layout asks of the cascade once it knows where an element's
/// content falls (M16): the style of an element or a pseudo-element
/// that inherits from something other than its parent or originating
/// element. A `::first-letter` inherits from the element its text is
/// in, or from the `::first-letter` around it (Pseudo-Elements 4
/// §2.2.3); a `::first-line` from the block whose line it is, and an
/// element on that line from it (§2.1.3). Only the box tree finds
/// these; `layout` defines no cascade of its own, so it asks through
/// this.
pub trait Recascade {
    /// The style of the element `element`, cascaded from its rules as
    /// [`Styles::get`]'s is, but inheriting from `parent`.
    fn element(&self, element: NodeId, parent: &ComputedStyle) -> ComputedStyle;

    /// The style of the element `element`'s `pseudo`, cascaded from its
    /// rules as [`Styles::pseudo`]'s is, but inheriting from `parent`;
    /// `None` when no rule styles it.
    fn pseudo_element(
        &self,
        element: NodeId,
        pseudo: PseudoElement,
        parent: &ComputedStyle,
    ) -> Option<ComputedStyle>;
}

/// [`Recascade`] over a document, the [`Stylesheets`] and context it was
/// cascaded with, and the styles that gave: an element's rules are
/// matched again for each question, which layout asks once per
/// `::first-letter` box and `::first-line`, and per inline box on a first
/// line's run — but for a pseudo-element of an element asked with the
/// element's own style as parent, whose style the cascade kept.
pub struct Recascader<'a> {
    sheets: &'a Stylesheets,
    document: &'a Document,
    styles: &'a Styles,
    cascader: RefCell<Cascader<'a>>,
}

impl Stylesheets {
    /// A [`Recascade`] over `document` and these sheets, with
    /// `context`, the cascade's, and `styles`, what it gave.
    pub fn recascader<'a>(
        &'a self,
        document: &'a Document,
        styles: &'a Styles,
        context: &'a CascadeContext<'a>,
    ) -> Recascader<'a> {
        Recascader {
            sheets: self,
            document,
            styles,
            cascader: RefCell::new(self.cascader(document, context)),
        }
    }
}

/// A declaration block that applied to an element (M36, the DevTools'
/// element inspector): its origin, its selector and specificity, the
/// conditional and layer rules around it, and its declarations.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchedRule {
    pub origin: RuleOrigin,
    /// The selector list as written, empty for a style attribute or
    /// presentational hints.
    pub selector: String,
    /// The highest specificity of the selectors of it that matched.
    pub specificity: (u32, u32, u32),
    /// The at-rules it is inside, outermost first: `@media ...`,
    /// `@supports ...`, `@layer ...`.
    pub conditions: Vec<String>,
    pub declarations: Vec<MatchedDeclaration>,
}

/// Where a matched rule is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleOrigin {
    /// The browser's own stylesheet.
    UserAgent,
    /// The document's stylesheet of this index among its sheets.
    Author(usize),
    /// HTML's presentational hints and SVG's presentation attributes.
    Hints,
    /// The element's `style` attribute.
    Inline,
}

/// A declaration of a matched rule: overridden when one later in the
/// cascade's order sets every longhand it sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedDeclaration {
    pub name: String,
    pub value: String,
    pub important: bool,
    pub overridden: bool,
}

impl Recascader<'_> {
    /// The declaration blocks that apply to `element` itself (its
    /// pseudo-elements aside), the first that wins first (M36): the
    /// style attribute's, then the rules by their cascade order, each
    /// declaration marked overridden where a later one sets what it
    /// sets.
    pub fn matched_rules(&self, element: NodeId) -> Vec<MatchedRule> {
        let mut cascader = self.cascader.borrow_mut();
        let cascader = &mut *cascader;
        let document = self.document;
        let svg = document.svg_tag(element).is_some();
        // Each block's declarations by cascade key: (tier, layer,
        // specificity, order, block, declaration).
        let mut blocks: Vec<MatchedRule> = Vec::new();
        let mut keys: Vec<(u8, u32, Specificity, u32, usize, usize)> = Vec::new();
        let mut order = 0u32;
        let ua = ua();
        for (in_force, author) in [(&cascader.ua, false), (&cascader.author, true)] {
            if author {
                let ua_important = ua_important_declarations(document, element);
                if !ua_important.is_empty() {
                    add_block(
                        &mut blocks,
                        &mut keys,
                        &mut order,
                        RuleOrigin::UserAgent,
                        String::new(),
                        Specificity::default(),
                        Vec::new(),
                        (TierBase::UserAgent, 0, 0),
                        &ua_important,
                    );
                }
                // The hints come before every author sheet's rules.
                let hints = hint_declarations(document, element, cascader.context.frame_margins);
                if !hints.is_empty() {
                    add_block(
                        &mut blocks,
                        &mut keys,
                        &mut order,
                        RuleOrigin::Hints,
                        String::new(),
                        Specificity::default(),
                        Vec::new(),
                        (TierBase::AuthorSheet, 0, 0),
                        &hints,
                    );
                }
            }
            in_force.candidates(document, element, &mut cascader.candidates);
            let candidates = std::mem::take(&mut cascader.candidates);
            for (place, specificity) in
                matching_rules(&mut cascader.matcher, (element, svg), in_force, &candidates)
            {
                let rule = &in_force[place as usize];
                let (sheet, selector, conditions) = if author {
                    self.sheets.rules.described(&self.sheets.sheets, rule.rule)
                } else {
                    ua.rules
                        .described(std::slice::from_ref(&ua.sheet), rule.rule)
                };
                let (origin, base) = if author {
                    (RuleOrigin::Author(sheet as usize), TierBase::AuthorSheet)
                } else {
                    (RuleOrigin::UserAgent, TierBase::UserAgent)
                };
                add_block(
                    &mut blocks,
                    &mut keys,
                    &mut order,
                    origin,
                    selector,
                    specificity,
                    conditions,
                    (base, rule.layer, rule.important_layer),
                    rule.declarations,
                );
            }
            cascader.candidates = candidates;
        }
        if let Some(style_attr) = document.attr(element, "style")
            && cascader
                .context
                .inline_styles
                .is_none_or(|inline| inline.allowed(element, style_attr))
            && let Ok(decls) = css::parse_style_attribute(style_attr)
        {
            add_block(
                &mut blocks,
                &mut keys,
                &mut order,
                RuleOrigin::Inline,
                String::new(),
                Specificity::default(),
                Vec::new(),
                (TierBase::AuthorInline, 0, 0),
                &decls,
            );
        }
        // From the winner down, a declaration is overridden when what it
        // sets is set already.
        keys.sort_by_key(|&(tier, layer, specificity, order, ..)| {
            (tier, layer, specificity, order)
        });
        let mut set: HashSet<String> = HashSet::new();
        for &(.., block, index) in keys.iter().rev() {
            let declaration = &mut blocks[block].declarations[index];
            let longhands: Vec<String> = match crate::animate::longhands_of(&declaration.name) {
                list if list.is_empty() => vec![declaration.name.clone()],
                list => list
                    .iter()
                    .map(|longhand| format!("{longhand:?}"))
                    .collect(),
            };
            let mut new = false;
            for longhand in longhands {
                new |= set.insert(longhand);
            }
            declaration.overridden = !new;
        }
        // The rules from the winner down, each by its first place in the
        // cascade's order: the style attribute first, then the most
        // specific, and so on.
        let mut ranked: Vec<(Option<CascadeKey>, MatchedRule)> = blocks
            .into_iter()
            .enumerate()
            .map(|(block, rule)| {
                let rank = keys
                    .iter()
                    .filter(|key| key.4 == block)
                    .map(|&(tier, layer, specificity, order, ..)| (tier, layer, specificity, order))
                    .min();
                (rank, rule)
            })
            .collect();
        ranked.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
        ranked.into_iter().map(|(_, rule)| rule).collect()
    }
}

/// A declaration's place in the cascade's order: its tier, layer,
/// specificity and order.
type CascadeKey = (u8, u32, Specificity, u32);

/// Adds the block of `declarations` to `blocks`, and each declaration's
/// cascade key to `keys`.
#[allow(clippy::too_many_arguments)]
fn add_block(
    blocks: &mut Vec<MatchedRule>,
    keys: &mut Vec<(u8, u32, Specificity, u32, usize, usize)>,
    order: &mut u32,
    origin: RuleOrigin,
    selector: String,
    specificity: Specificity,
    conditions: Vec<String>,
    (base, layer, important_layer): (TierBase, u32, u32),
    declarations: &[css::Declaration],
) {
    let block = blocks.len();
    for (index, decl) in declarations.iter().enumerate() {
        let layer = if decl.important {
            important_layer
        } else {
            layer
        };
        keys.push((
            tier(base, decl.important),
            layer,
            specificity,
            *order,
            block,
            index,
        ));
        *order += 1;
    }
    blocks.push(MatchedRule {
        origin,
        selector,
        specificity: (specificity.0, specificity.1, specificity.2),
        conditions,
        declarations: declarations
            .iter()
            .map(|decl| MatchedDeclaration {
                name: decl.name.clone(),
                value: crate::serialize::tokens(&decl.value),
                important: decl.important,
                overridden: false,
            })
            .collect(),
    });
}

/// The rules of `sheet` whose selectors match `element` itself, by their
/// places, each with the highest specificity of those that matched, of
/// `candidates` only, as [`collect_from_sheet`] tries them.
fn matching_rules<'a>(
    matcher: &mut Matcher<'a>,
    (element, svg): (NodeId, bool),
    sheet: &InForce<'a>,
    candidates: &[(u32, u32)],
) -> Vec<(u32, Specificity)> {
    let mut out = Vec::new();
    let mut next = 0;
    while next < candidates.len() {
        let place = candidates[next].0;
        let start = next;
        while next < candidates.len() && candidates[next].0 == place {
            next += 1;
        }
        let rule = &sheet[place as usize];
        if rule.svg_only && !svg {
            continue;
        }
        let roots = rule
            .scope
            .map(|scope| scoping_roots(matcher, sheet, scope, element));
        let list: &'a SelectorList = rule.list;
        let mut best: Option<Specificity> = None;
        for &(_, index) in &candidates[start..next] {
            let selector = &list.0[index as usize];
            if selector.pseudo_element.is_some() {
                continue;
            }
            let matches = match &roots {
                None => matcher.matches_element(element, selector),
                Some(roots) => roots.iter().any(|&(root, _)| {
                    matcher.set_rule_scope(Some(root));
                    let found = matcher.matches_element(element, selector);
                    matcher.set_rule_scope(None);
                    found
                }),
            };
            if matches {
                best = best.max(Some(selector.specificity));
            }
        }
        if let Some(specificity) = best {
            out.push((place, specificity));
        }
    }
    out
}

impl Recascader<'_> {
    /// `cascade` of `element`'s matched declarations, resolved as the
    /// cascade resolves them: the root element's `rem` is the initial
    /// font size, every other element's the root's.
    fn with_matched<R>(
        &self,
        element: NodeId,
        cascade: impl FnOnce(&Matched<'_>, &Resolution<'_>) -> R,
    ) -> R {
        dom::hook!("style::with_matched");
        self.with_matched_for(element, None, cascade)
    }

    /// [`Recascader::with_matched`], or with `highlight` the declarations
    /// of the rules for `element`'s `::highlight()` of that name alone,
    /// in the element's place.
    fn with_matched_for<R>(
        &self,
        element: NodeId,
        highlight: Option<HighlightName>,
        cascade: impl FnOnce(&Matched<'_>, &Resolution<'_>) -> R,
    ) -> R {
        dom::hook!("style::with_matched_for");
        let mut cascader = self.cascader.borrow_mut();
        let cascader = &mut *cascader;
        let containers = Containers {
            document: self.document,
            styles: self.styles,
            sizes: &self.styles.container_sizes,
            queried: &cascader.queried,
            substitutions: &cascader.substitutions,
            viewport: (
                cascader.context.viewport_width,
                cascader.context.viewport_height,
            ),
            rem: self.styles.rem,
            fonts: cascader.context.fonts,
            own: None,
        };
        let matched = match highlight {
            None => declarations_for(
                self.document,
                (&mut cascader.matcher, &mut cascader.candidates),
                element,
                (&cascader.ua, &cascader.author, &cascader.shadow),
                (cascader.context.inline_styles, cascader.context.top_layer),
                cascader.context.frame_margins,
                Some(&containers),
            ),
            Some(name) => highlight_declarations(
                self.document,
                (&mut cascader.matcher, &mut cascader.candidates),
                element,
                (&cascader.ua, &cascader.author, &cascader.shadow),
                name,
                Some(&containers),
            ),
        };
        let rem = if self.document.document_element() == Some(element) {
            ComputedStyle::default().font_size_px
        } else {
            self.styles.rem
        };
        let resolution = Resolution {
            rem,
            root: self.document.document_element() == Some(element),
            page_scheme: cascader.page_scheme.as_deref().map(String::as_str),
            context: cascader.context,
            viewport_used: &cascader.viewport_used,
            fonts_used: Some(&cascader.fonts_used),
            root_font: self
                .styles
                .root_font
                .as_deref()
                .filter(|_| self.document.document_element() != Some(element)),
            root_size: rem,
            scheme_read: Some(&cascader.scheme_read),
            attributes: self.document.attrs(element).unwrap_or_default(),
            substitutions: &cascader.substitutions,
            containers: Some(&containers),
            units_from: self.document.flat_parent_element(element),
            containers_used: &cascader.containers_used,
        };
        cascade(&matched, &resolution)
    }
}

impl Recascader<'_> {
    /// The style of `element`'s `::highlight(name)` (CSS Pseudo 4 section
    /// 3.5, CSS Highlight API section 3, gap batch H), cascaded from its
    /// rules and inheriting from `parent`: the same highlight's style of
    /// the element's parent, as highlights inherit, or [`highlight_root`]
    /// for the root element. `None` when no rule styles it, the style
    /// then being `parent`'s inherited values.
    pub fn highlight(
        &self,
        element: NodeId,
        name: HighlightName,
        parent: &ComputedStyle,
    ) -> Option<ComputedStyle> {
        dom::hook!("style::highlight");
        self.with_matched_for(element, Some(name), |matched, resolution| {
            let applied = matched.of(Target::Element);
            (!applied.is_empty())
                .then(|| compute_style(applied, parent, Target::Highlight, resolution, None, None))
        })
    }
}

/// What the root element's highlights inherit from (CSS Pseudo 4
/// section 3.5): the root's own style, its `color` left as
/// `currentcolor`, so that a highlight that sets no colour draws its
/// text in the colour under it, as the reference browser does.
pub fn highlight_root(root: &ComputedStyle) -> ComputedStyle {
    let mut style = ComputedStyle::inherited_from(root);
    style.color = Color::CURRENT;
    style
}

impl Recascade for Recascader<'_> {
    fn element(&self, element: NodeId, parent: &ComputedStyle) -> ComputedStyle {
        let base = self.with_matched(element, |matched, resolution| {
            element_style(
                self.document,
                element,
                matched,
                parent,
                resolution,
                None,
                None,
            )
        });
        // Its animations and transitions at the timeline's time, as the
        // cascade applied them, none started, matched again or cancelled.
        let animating = self.cascader.borrow().sample((element, None), &base, None);
        let parent = Parent {
            style: parent,
            after_change: None,
            display_none: false,
            unrendered: false,
        };
        let animations = self.cascader.borrow().context.animations;
        changed_style(
            animations,
            (element, None),
            base,
            animating.as_ref(),
            parent,
            Before::Unknown,
            |_| true,
            |parent, animated, transitioned| {
                self.with_matched(element, |matched, resolution| {
                    element_style(
                        self.document,
                        element,
                        matched,
                        parent,
                        resolution,
                        animated,
                        transitioned,
                    )
                })
            },
            |_, _| None,
        )
        .0
    }

    fn pseudo_element(
        &self,
        element: NodeId,
        pseudo: PseudoElement,
        parent: &ComputedStyle,
    ) -> Option<ComputedStyle> {
        let target = Target::of(pseudo)?;
        // From the element's own style, it is what the cascade kept: no
        // rule need be matched again.
        if self.styles.get(element) == Some(parent) {
            return self.styles.pseudo(element, pseudo).cloned();
        }
        // A list item's marker has a style whether a rule styles it or
        // not, and nothing else has one.
        let marker = target == Target::Marker;
        if marker
            && !self
                .styles
                .get(element)
                .is_some_and(|s| s.display.is_list_item())
        {
            return None;
        }
        let base = self.with_matched(element, |matched, resolution| {
            let applied = matched.of_pseudo(target, element, parent, resolution.containers);
            (marker || !applied.is_empty())
                .then(|| compute_style(&applied, parent, target, resolution, None, None))
        })?;
        let Some(pseudo) = target.animated() else {
            return Some(base);
        };
        let target_of = (element, Some(pseudo));
        let animating = self.cascader.borrow().sample(target_of, &base, None);
        let parent = Parent {
            style: parent,
            after_change: None,
            display_none: false,
            unrendered: false,
        };
        let animations = self.cascader.borrow().context.animations;
        let style = changed_style(
            animations,
            target_of,
            base,
            animating.as_ref(),
            parent,
            Before::Unknown,
            |_| true,
            |parent, animated, transitioned| {
                self.with_matched(element, |matched, resolution| {
                    let own = self.styles.get(element).unwrap_or(parent);
                    compute_style(
                        &matched.of_pseudo(target, element, own, resolution.containers),
                        parent,
                        target,
                        resolution,
                        animated,
                        transitioned,
                    )
                })
            },
            |_, _| None,
        )
        .0;
        Some(style)
    }
}

/// An element's style from its `matched` declarations, inheriting from
/// `parent`, with the values of its animations in effect, `animated`,
/// and of its running transitions, `transitioned`: its `dir` attribute
/// read, the root element's and a legend's `display` blockified.
fn element_style(
    document: &Document,
    element: NodeId,
    matched: &Matched<'_>,
    parent: &ComputedStyle,
    resolution: &Resolution<'_>,
    animated: Option<&Animated<'_>>,
    transitioned: Option<&[Transitioning]>,
) -> ComputedStyle {
    dom::hook!("style::element_style");
    // A MathML script, or a part of a fraction, inherits a smaller font
    // and the compact style (D77).
    let scripted;
    let parent = match script_level(document, element, parent) {
        Some(scripted_parent) => {
            scripted = scripted_parent;
            &scripted
        }
        None => parent,
    };
    let mut style = compute_style(
        matched.of(Target::Element),
        parent,
        Target::Element,
        resolution,
        animated,
        transitioned,
    );
    // The root element's box is a block container or a table, whatever
    // its `display` (CSS2 §9.7, css-display-3 §2.7): what the canvas
    // takes its background from, and every other box is inside.
    if document.document_element() == Some(element) {
        style.display = match style.display {
            // The root's `contents` is `block` (css-display-3 section
            // 2.5).
            Display::Contents => Display::Block,
            other => other.blockified(),
        };
        style.root = true;
    }
    // `math` is MathML's elements' alone: on another it computes to the
    // flow value (MathML Core section 2.1).
    if document.namespace(element) != Some(&dom::Namespace::MathMl) {
        style.display = style.display.without_math();
    } else {
        set_math_element(document, element, &mut style);
        // `semantics` shows its first child alone, the others being its
        // annotations (MathML Core section 3.7).
        if let Some(up) = document.parent_element(element)
            && document.namespace(up) == Some(&dom::Namespace::MathMl)
            && document.local_name(up) == Some("semantics")
            && document.element_children(up).next() != Some(element)
        {
            style.display = Display::None;
        }
    }
    // An element whose box is a replaced element's, a form control's or
    // a frame's, or SVG's other than its containers', makes none with
    // `contents`, as css-display-3's appendix B and the reference
    // browser have it.
    if style.display == Display::Contents && unboxes_to_nothing(document, element) {
        style.display = Display::None;
    }
    // A `legend` is blockified wherever it is, as in the reference
    // browser; a `fieldset` is marked for layout (HTML section 15.3.13).
    match document.tag(element) {
        Some("legend") => style.display = style.display.blockified(),
        Some("fieldset") => style.fieldset = true,
        _ => {}
    }
    if let Some(lang) = document.attr(element, "lang") {
        set_language(&mut style, lang);
    }
    top_layer::adjust(document, element, &mut style);
    // Whether it skips its contents (CSS Containment 2 section 4.1): an
    // `auto` one by its relevance to the user, which the engine keeps
    // as a state of the element (gap batch H).
    let skips = style.would_skip_contents(relevant(document, element));
    style.set_skips_contents(skips);
    style
}

/// Whether `element` is relevant to the user, as the engine last found
/// (CSS Containment 2 section 4.3).
fn relevant(document: &Document, element: NodeId) -> bool {
    document
        .state(element)
        .contains(dom::ElementState::RELEVANT)
}

/// The MathML Core layout the MathML element `element` takes (D77), its
/// attributes read for it: `mspace`'s `height` and `depth`, and its
/// `width` as its CSS width where no rule gives one, and `mfrac`'s
/// `linethickness`, each a length (MathML Core section 2.2.1), invalid
/// ones dropped.
fn set_math_element(document: &Document, element: NodeId, style: &mut ComputedStyle) {
    dom::hook!("style::set_math_element");
    use crate::computed::MathElement;
    let em = style.font_size_px;
    let length = |name: &str| {
        let tokens = css::tokenize(document.attr(element, name)?);
        let px = match single_token(&tokens)? {
            Token::Dimension { value, unit } => {
                let unit = LengthUnit::of(unit)?;
                match unit.absolute() {
                    Some(per) => value * per,
                    None if unit == LengthUnit::Em => value * f64::from(em),
                    // An `ex` is half an em here, as the font is not at
                    // hand.
                    None if unit == LengthUnit::Ex => value * f64::from(em) / 2.0,
                    None => return None,
                }
            }
            Token::Number { value, .. } if *value == 0.0 => 0.0,
            _ => return None,
        };
        px.is_finite().then_some(px as f32)
    };
    style.text_mut().math = match document.local_name(element).unwrap_or_default() {
        "mi" | "mn" | "mtext" | "ms" => MathElement::Token,
        "mo" => MathElement::Operator,
        "mspace" => {
            if style.width == Size::Auto
                && let Some(width) = length("width").filter(|w| *w >= 0.0)
            {
                style.width = Size::Px(width);
            }
            MathElement::Space {
                height: length("height").unwrap_or(0.0).max(0.0),
                depth: length("depth").unwrap_or(0.0).max(0.0),
            }
        }
        "mfrac" => MathElement::Fraction {
            thickness: length("linethickness").map(|t| t.max(0.0)),
        },
        "msqrt" => MathElement::Sqrt,
        "mroot" => MathElement::Root,
        "msub" => MathElement::Sub,
        "msup" => MathElement::Sup,
        "msubsup" => MathElement::SubSup,
        "munder" => MathElement::Under,
        "mover" => MathElement::Over,
        "munderover" => MathElement::UnderOver,
        _ => MathElement::Row,
    };
}

/// The style a MathML element inherits in place of `parent` when it is a
/// script or a part of a fraction (MathML Core section 4's UA sheet, its
/// `math-depth` and `math-style`, D77): every child of an `mfrac`, and
/// every child but the first of the scripted and the limit elements, is
/// `compact`, and set at `math-depth` one more than its parent's, a font
/// 71% of its parent's (the MATH table's scriptPercentScaleDown's
/// fallback), where its parent is compact, as `auto-add` makes it for a
/// fraction's, or always, as `add(1)` makes it for a script's; `mroot`'s
/// index two more. `None` for any other element, which inherits its
/// parent's style as it is.
fn script_level(
    document: &Document,
    element: NodeId,
    parent: &ComputedStyle,
) -> Option<ComputedStyle> {
    dom::hook!("style::script_level");
    use crate::computed::MathStyle;
    if document.namespace(element) != Some(&dom::Namespace::MathMl) {
        return None;
    }
    let up = document.parent_element(element)?;
    if document.namespace(up) != Some(&dom::Namespace::MathMl) {
        return None;
    }
    let first = document.element_children(up).next() == Some(element);
    let levels = match document.local_name(up)? {
        "mfrac" if parent.text().math_style == MathStyle::Compact => 1,
        "mfrac" => 0,
        "mroot" if !first => 2,
        "msub" | "msup" | "msubsup" | "munder" | "mover" | "munderover" if !first => 1,
        _ => return None,
    };
    let mut scripted = parent.clone();
    scripted.font_size_px *= 0.71f32.powi(levels);
    scripted.text_mut().math_style = MathStyle::Compact;
    Some(scripted)
}

/// Whether `element` makes no box at all with `display: contents`
/// (css-display-3 appendix B, D77): HTML's replaced elements, form
/// controls and frames, whose content is not their children's boxes;
/// and an SVG element but for those that only group others (`a`, `g`,
/// `switch`, `use`, `tspan`, `textPath` and an inner `svg`), the outer
/// `svg` among the others.
fn unboxes_to_nothing(document: &Document, element: NodeId) -> bool {
    dom::hook!("style::unboxes_to_nothing");
    if let Some(tag) = document.svg_tag(element) {
        let outer = document
            .parent_element(element)
            .is_none_or(|parent| document.svg_tag(parent).is_none());
        return outer
            || !matches!(
                tag,
                "a" | "g" | "switch" | "use" | "tspan" | "textPath" | "svg"
            );
    }
    matches!(
        document.tag(element),
        Some(
            "br" | "wbr"
                | "meter"
                | "progress"
                | "canvas"
                | "embed"
                | "object"
                | "audio"
                | "iframe"
                | "img"
                | "video"
                | "frame"
                | "frameset"
                | "input"
                | "textarea"
                | "select"
        )
    )
}

/// Takes `lang`, an element's `lang` attribute, as its content language,
/// which its descendants inherit: an empty one is unknown.
fn set_language(style: &mut ComputedStyle, lang: &str) {
    let lang = lang.trim().to_ascii_lowercase();
    let now = style.text().lang.as_deref();
    if now == Some(lang.as_str()) || (now.is_none() && lang.is_empty()) {
        return;
    }
    style.text_mut().lang = Some(Arc::from(lang));
}

/// The animations `motion` asks for (Animations 1 §2): an item of its
/// `animation-name` that names keyframes in force, with the other
/// properties' items at its place; an animation's effect is linear, its
/// timing function its keyframes'.
fn wanted<'k>(
    motion: &Motion,
    keyframes: impl Fn(&str) -> Option<KeyframesInForce<'k>>,
) -> Vec<Wanted> {
    motion
        .animations()
        .filter_map(|item| {
            let name = item.name?;
            let found = keyframes(name)?;
            Some(Wanted {
                name: Arc::clone(name),
                // The effect's timing is in milliseconds.
                duration_auto: item.duration.is_none(),
                timeline: item.timeline.clone(),
                range: [item.range_start.clone(), item.range_end.clone()],
                timing: Timing {
                    delay: item.delay * 1000.0,
                    duration: item.duration.unwrap_or(0.0) * 1000.0,
                    iterations: item.iteration_count,
                    direction: item.direction,
                    fill: item.fill_mode,
                    ..Timing::default()
                },
                easing: item.timing_function.clone(),
                paused: item.play_state == AnimationPlayState::Paused,
                composite: item.composition,
                longhands: Arc::from(found.keyframes.longhands.as_slice()),
            })
        })
        .collect()
}

/// What [`Stylesheets::restyle`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Restyle {
    /// The document was cascaded whole: every element counts as
    /// changed, and [`Restyle::changed`] is empty.
    pub full: bool,
    /// The elements whose computed style changed, a newly styled one
    /// among them, in the order they were restyled — but for those whose
    /// changes are composited.
    pub changed: Vec<NodeId>,
    /// The changes of style that are composited (M22.4): of an element,
    /// or a pseudo-element of it, in its transform or its opacity alone,
    /// leaving its box what it was to layout.
    pub composited: Vec<Composited>,
    /// How many elements had their style computed.
    pub computed: usize,
}

/// A composited change of style (M22.4): of an element's, or its
/// pseudo-element's, `transform`, `translate`, `rotate`, `scale` or
/// `opacity` alone, which leaves its box what it was to layout — a
/// stacking context or not, a containing block for absolutely and fixed
/// positioned boxes or not, flattening its 3D content or not. Nothing
/// inherits those properties, and layout reads none of them: the
/// caller can give the boxes the new effects rather than lay them out.
#[derive(Debug, Clone, PartialEq)]
pub struct Composited {
    pub node: NodeId,
    pub pseudo: Option<PseudoElement>,
    /// The effects the style it replaces had, which its boxes carry.
    pub old: Arc<Effects>,
    /// Its new effects.
    pub new: Arc<Effects>,
    /// Whether its transform changed, which can change the scrollable
    /// overflow its boxes are part of.
    pub transformed: bool,
    /// The style it replaces and the new one, which differ in these
    /// effects alone (T4.3, M35): a box that carries the very style
    /// takes the new one rather than a copy of its own.
    pub old_style: Arc<ComputedStyle>,
    pub new_style: Arc<ComputedStyle>,
}

/// The composited changes from `old` to `new`, the computed styles of the
/// element `node` before a change and after it, when every change is one;
/// `None` when any is not, an element's own or a pseudo-element's.
fn composited_change(node: NodeId, old: &Computed, new: &Computed) -> Option<Vec<Composited>> {
    if old.display_none != new.display_none || old.unrendered != new.unrendered {
        return None;
    }
    let mut out = Vec::new();
    // The after-change styles are the children's to read, not layout's.
    composited_style(node, None, &old.style, &new.style, &mut out)?;
    match (old.pseudo.as_deref(), new.pseudo.as_deref()) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            // Every pseudo-element style kept, named so that one kept
            // later is not passed over.
            let PseudoStyles {
                before,
                after,
                first_letter,
                first_line,
                marker,
                backdrop,
                details_content,
                details_content_before,
                details_content_after,
            } = a;
            for (pseudo, a, b) in [
                (PseudoElement::Before, before, &b.before),
                (PseudoElement::After, after, &b.after),
                (PseudoElement::FirstLetter, first_letter, &b.first_letter),
                (PseudoElement::FirstLine, first_line, &b.first_line),
                (PseudoElement::Marker, marker, &b.marker),
                (PseudoElement::Backdrop, backdrop, &b.backdrop),
                (
                    PseudoElement::DetailsContent,
                    details_content,
                    &b.details_content,
                ),
                (
                    PseudoElement::DetailsContentBefore,
                    details_content_before,
                    &b.details_content_before,
                ),
                (
                    PseudoElement::DetailsContentAfter,
                    details_content_after,
                    &b.details_content_after,
                ),
            ] {
                match (a, b) {
                    (None, None) => {}
                    (Some(a), Some(b)) => composited_style(node, Some(pseudo), a, b, &mut out)?,
                    _ => return None,
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

/// Adds to `out` the composited change from `old` to `new`, styles of
/// `node`'s `pseudo`, if they differ; `None` when they differ otherwise.
/// A style with no effects of its own is not composited from: no box
/// can be told to carry them.
fn composited_style(
    node: NodeId,
    pseudo: Option<PseudoElement>,
    old: &Arc<ComputedStyle>,
    new: &Arc<ComputedStyle>,
    out: &mut Vec<Composited>,
) -> Option<()> {
    if old == new {
        return Some(());
    }
    let (Some(a), Some(b)) = (&old.effects, &new.effects) else {
        return None;
    };
    let without = |style: &ComputedStyle| ComputedStyle {
        effects: None,
        ..style.clone()
    };
    let blank = |effects: &Effects| Effects {
        transform: Vec::new(),
        translate: None,
        rotate: None,
        scale: None,
        opacity: 1.0,
        // What a transition or an animation acts as if `will-change`
        // named (G33) is layout's only where it makes a stacking context
        // or a containing block, which the checks below compare.
        animated: WillChange::AUTO,
        ..effects.clone()
    };
    if without(old) != without(new) || blank(a) != blank(b) {
        return None;
    }
    // The rest of the effects equal, whether the box has a transform,
    // is a stacking context and contains its absolutely and fixed
    // positioned descendants is what layout reads of them, as a box the
    // transforms apply to or, for an `inline` element's, which may be an
    // inline box, as one they do not (css-transforms-1 section 3); and an
    // opacity can flatten its 3D content.
    let inline = old.display == Display::Inline || new.display == Display::Inline;
    let same_as = |transformable: bool| {
        a.is_stacking_context_of(transformable) == b.is_stacking_context_of(transformable)
            && a.contains_all_of(transformable) == b.contains_all_of(transformable)
    };
    let same_to_layout = a.has_transform() == b.has_transform()
        && same_as(true)
        && (!inline || same_as(false))
        && a.used_transform_style(old.overflow()) == b.used_transform_style(new.overflow());
    if !same_to_layout {
        return None;
    }
    out.push(Composited {
        node,
        pseudo,
        old: Arc::clone(a),
        new: Arc::clone(b),
        transformed: a.transform != b.transform
            || a.translate != b.translate
            || a.rotate != b.rotate
            || a.scale != b.scale,
        old_style: Arc::clone(old),
        new_style: Arc::clone(new),
    });
    Some(())
}

impl Restyle {
    /// A full cascade, which computed every element of `styles`.
    fn full(styles: &Styles) -> Self {
        Restyle {
            full: true,
            changed: Vec::new(),
            composited: Vec::new(),
            computed: styles.entries.iter().flatten().count().saturating_sub(1),
        }
    }
}

/// The built-in user-agent sheet, prepared once per process: the
/// sheet, its rules, and what their selectors depend on.
struct UserAgent {
    sheet: css::Stylesheet,
    rules: RuleSet,
    features: SelectorFeatures,
}

fn ua() -> &'static UserAgent {
    static UA: OnceLock<UserAgent> = OnceLock::new();
    UA.get_or_init(|| {
        let sheet = css::parse_stylesheet(UA_STYLESHEET_SRC)
            .expect("the built-in UA stylesheet (assets/ua.css) is valid CSS");
        let mut features = SelectorFeatures::default();
        let rules = RuleSet::user_agent(std::slice::from_ref(&sheet), &mut features);
        UserAgent {
            sheet,
            rules,
            features,
        }
    })
}

/// Whether an element's children, styled under `old`, its computed
/// style before a change, can be styled otherwise under `new` (G254):
/// through what they inherit of its style and of its after-change style
/// (which their own after-change styles are made from), its display (a
/// flex or a grid container's items are blockified), or whether it is
/// rendered. What a child that inherits a property no child otherwise
/// inherits reads of it the caller asks apart.
fn reaches_children(old: &Computed, new: &Computed) -> bool {
    let inherited = |style: &ComputedStyle| ComputedStyle::inherited_from(style);
    let after = |computed: &Computed| computed.after_change.as_deref().map(inherited);
    old.display_none != new.display_none
        || old.unrendered != new.unrendered
        || old.style.display != new.style.display
        || old.style.contents_in_items != new.style.contents_in_items
        || inherited(&old.style) != inherited(&new.style)
        || after(old) != after(new)
}

/// The most base styles a cascade keeps for the elements after made of
/// the same (M35.2): a few megabytes; past it they are forgotten and
/// computed again.
pub const MAX_KEPT_STYLES: usize = 4096;

/// How many distinct parent styles a cascade remembers, to number the
/// parents of equal styles alike, so that cousins share what they make.
const RECENT_PARENTS: usize = 8;

/// The longhands an animation of which may be set on the style made
/// without it (M35.2): those nothing else in a style is computed from,
/// what the compositor draws.
const COMPOSITED_LONGHANDS: [Longhand; 5] = [
    Longhand::Opacity,
    Longhand::Transform,
    Longhand::Translate,
    Longhand::Rotate,
    Longhand::Scale,
];

#[cfg(test)]
thread_local! {
    /// Whether the cascades made on this thread keep no style for the
    /// elements after and set no animation on a style (M35.2): the
    /// tests' reference, what is kept compared with.
    pub(crate) static PLAIN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How many base styles the cascades made on this thread computed
    /// rather than took from those kept.
    pub(crate) static AFRESH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// How many animated styles were made by setting their animations'
    /// values on the style made without them.
    pub(crate) static SET_ON: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Whether this cascade keeps nothing between elements: the tests'
/// reference alone.
fn plain() -> bool {
    #[cfg(test)]
    {
        PLAIN.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        false
    }
}

/// What an element's base style is made of (M35.2): the number of its
/// parent's style among the distinct ones met, whether it is the root,
/// its `dir`, the `rem` it resolves against, and each matched
/// declaration by address, with its tier, layer, specificity, scope
/// proximity and order.
#[derive(PartialEq, Eq, Hash)]
struct StyleKey {
    /// Whether the element makes no box with `display: contents` (D77).
    unboxes: bool,
    /// Whether the element is relevant to the user, which a
    /// `content-visibility: auto` reads (gap batch H).
    relevant: bool,
    parent: u32,
    root: bool,
    dir: Option<Box<str>>,
    lang: Option<Box<str>>,
    rem: u32,
    /// Each declaration's address, tier, layer, specificity with its
    /// context, scope proximity and order.
    declarations: Vec<(usize, u8, u32, [u32; 4], u32, u32)>,
}

/// The cascade's working state over one document: the matcher, with
/// its caches, and the rules in force.
struct Cascader<'a> {
    document: &'a Document,
    matcher: Matcher<'a>,
    ua: InForce<'a>,
    author: InForce<'a>,
    /// Each shadow tree's rules in force, by its root (D69).
    shadow: HashMap<NodeId, InForce<'a>>,
    /// The selectors an element is tried against, made again for each
    /// (M35.2): kept to spare an allocation an element.
    candidates: Vec<(u32, u32)>,
    /// The base styles made in this cascade, by what they are made of
    /// (M35.2), at most [`MAX_KEPT_STYLES`]: an element made of the same
    /// takes the one made, rather than computing it again. Off for the
    /// tests that compare.
    kept: HashMap<StyleKey, Arc<ComputedStyle>>,
    keeps: bool,
    /// Each parent's style's number, by node, and the last distinct
    /// parent styles with theirs.
    parent_numbers: HashMap<NodeId, u32>,
    recent_parents: VecDeque<(u32, ComputedStyle)>,
    /// Whether each declaration met, by address, reads an attribute.
    reads_attributes: HashMap<usize, bool>,
    /// Whether an animation of composited longhands alone is set on the
    /// style made without it rather than computing it again (M35.2). Off
    /// for the tests that compare.
    sets_composited: bool,
    context: &'a CascadeContext<'a>,
    /// The answers that left these rules in force, kept with the styles.
    media: Vec<bool>,
    /// Set when a viewport unit is met.
    viewport_used: Cell<bool>,
    /// Set when a unit a font's figures give is met.
    fonts_used: Cell<bool>,
    /// The root element's font, once it is computed.
    root_font: Option<Arc<FontProps>>,
    /// Set when the user's colour scheme chose an element's (D49).
    scheme_read: Cell<bool>,
    /// What the substitutions of this cascade share (M21.1).
    substitutions: Substitutions,
    /// The `@keyframes` rules in force by name, when the context runs
    /// animations (M22.2).
    keyframes: HashMap<&'a str, KeyframesInForce<'a>>,
    /// The shadow trees' `@keyframes` rules in force by name, by their
    /// roots (D69).
    shadow_keyframes: HashMap<NodeId, HashMap<&'a str, KeyframesInForce<'a>>>,
    /// The document's `color-scheme` meta element's value (gap batch E,
    /// D49), what `normal` is.
    page_scheme: Option<Arc<String>>,
    /// The size containers' content boxes the queries read (D72).
    container_sizes: ContainerSizes,
    /// The containers the queries and the container units asked about.
    queried: RefCell<Queried>,
    /// Set when an element's style meets a container unit.
    containers_used: Cell<bool>,
    /// The author rules of an element's starting style (Transitions 2
    /// section 3.1), `@starting-style`'s among them, where there are any.
    starting: Option<InForce<'a>>,
}

impl<'a> Cascader<'a> {
    /// A cascader with `ua` and `author`, rules of the stylesheets
    /// numbered `sheets`.
    fn new(
        document: &'a Document,
        ua: InForce<'a>,
        author: InForce<'a>,
        context: &'a CascadeContext<'a>,
        media: Vec<bool>,
        sheets: u64,
    ) -> Self {
        Cascader {
            document,
            matcher: Matcher::with_language(document, context.language),
            ua,
            author,
            shadow: HashMap::new(),
            candidates: Vec::new(),
            kept: HashMap::new(),
            keeps: !plain(),
            parent_numbers: HashMap::new(),
            recent_parents: VecDeque::new(),
            reads_attributes: HashMap::new(),
            sets_composited: !plain(),
            context,
            page_scheme: page_scheme(document),
            media,
            viewport_used: Cell::new(false),
            fonts_used: Cell::new(false),
            root_font: None,
            scheme_read: Cell::new(false),
            substitutions: Substitutions::new(sheets),
            keyframes: HashMap::new(),
            container_sizes: ContainerSizes::new(),
            queried: RefCell::new(Queried::new()),
            containers_used: Cell::new(false),
            shadow_keyframes: HashMap::new(),
            starting: None,
        }
    }

    /// The computed style of `element`, a child of `parent`, with `rem`
    /// the root element's font size, and `before` what is known of its
    /// style before this change. Its animations, and its
    /// pseudo-elements', are matched to what their styles ask for
    /// (M22.2), and their transitions started, replaced and cancelled as
    /// the change asks (M22.1).
    /// `styles` holds the styles of its ancestors, which its container
    /// queries and units read (D72).
    #[allow(clippy::too_many_arguments)]
    fn style_of(
        &mut self,
        element: NodeId,
        parent: Parent<'_>,
        rem: f32,
        before: Before<'_>,
        old: Option<&Computed>,
        styles: &Styles,
    ) -> Computed {
        dom::hook!("style::style_of");
        let viewport = (self.context.viewport_width, self.context.viewport_height);
        let matched = {
            let containers = Containers {
                document: self.document,
                styles,
                sizes: &self.container_sizes,
                queried: &self.queried,
                substitutions: &self.substitutions,
                viewport,
                rem,
                fonts: self.context.fonts,
                own: None,
            };
            declarations_for(
                self.document,
                (&mut self.matcher, &mut self.candidates),
                element,
                (&self.ua, &self.author, &self.shadow),
                (self.context.inline_styles, self.context.top_layer),
                self.context.frame_margins,
                Some(&containers),
            )
        };
        let key = self.style_key(element, &matched, parent.style, rem);
        let containers = Containers {
            document: self.document,
            styles,
            sizes: &self.container_sizes,
            queried: &self.queried,
            substitutions: &self.substitutions,
            viewport,
            rem,
            fonts: self.context.fonts,
            own: None,
        };
        let resolution = Resolution {
            rem,
            root: self.document.document_element() == Some(element),
            page_scheme: self.page_scheme.as_deref().map(String::as_str),
            context: self.context,
            viewport_used: &self.viewport_used,
            fonts_used: Some(&self.fonts_used),
            root_font: self
                .root_font
                .as_deref()
                .filter(|_| self.document.document_element() != Some(element)),
            root_size: rem,
            scheme_read: Some(&self.scheme_read),
            attributes: self.document.attrs(element).unwrap_or_default(),
            substitutions: &self.substitutions,
            containers: Some(&containers),
            units_from: self.document.flat_parent_element(element),
            containers_used: &self.containers_used,
        };
        // A pseudo-element's query containers are its originating
        // element's inclusive ancestors.
        let pseudo_resolution = Resolution {
            units_from: Some(element),
            ..resolution
        };
        self.containers_used.set(false);
        let kept = key.as_ref().and_then(|key| self.kept.get(key)).cloned();
        let (base, shared) = match kept {
            Some(kept) => ((*kept).clone(), Some(kept)),
            None => {
                #[cfg(test)]
                AFRESH.with(|afresh| afresh.set(afresh.get() + 1));
                let base = element_style(
                    self.document,
                    element,
                    &matched,
                    parent.style,
                    &resolution,
                    None,
                    None,
                );
                // A style of container units is the element's own (D72).
                let shared = key.filter(|_| !self.containers_used.get()).map(|key| {
                    if self.kept.len() >= MAX_KEPT_STYLES {
                        self.kept.clear();
                    }
                    let shared = Arc::new(base.clone());
                    self.kept.insert(key, shared.clone());
                    shared
                });
                (base, shared)
            }
        };
        let important = matched
            .of(Target::Element)
            .iter()
            .any(|applied| applied.tier >= tier(TierBase::AuthorSheet, true));
        let sets_composited = self.sets_composited;
        let base_display = base.display;
        let display_none = parent.display_none || base_display == Display::None;
        // An element newly rendered has no before-change style: its
        // starting style stands for one (Transitions 2 section 3.1), its
        // `@starting-style` rules matched with the others, so that its
        // first style transitions from them.
        // One no style held before, which a cascade knows nothing of, is
        // newly rendered too.
        let starting_style;
        let newly = match before {
            Before::Known(None) => true,
            Before::Unknown => old.is_none(),
            Before::Known(Some(_)) => false,
        };
        let before = match (&before, &self.starting) {
            (_, Some(starting)) if newly && !display_none && transitions_run(&base) => {
                let containers = Containers {
                    document: self.document,
                    styles,
                    sizes: &self.container_sizes,
                    queried: &self.queried,
                    substitutions: &self.substitutions,
                    viewport,
                    rem,
                    fonts: self.context.fonts,
                    own: None,
                };
                let matched = declarations_for(
                    self.document,
                    (&mut self.matcher, &mut self.candidates),
                    element,
                    (&self.ua, starting, &self.shadow),
                    (self.context.inline_styles, self.context.top_layer),
                    self.context.frame_margins,
                    Some(&containers),
                );
                starting_style = element_style(
                    self.document,
                    element,
                    &matched,
                    parent.style,
                    &resolution,
                    None,
                    None,
                );
                // A starting style that renders nothing is none.
                if starting_style.display == Display::None {
                    Before::Known(None)
                } else {
                    Before::Known(Some(&starting_style))
                }
            }
            _ => before,
        };
        let animating = self.sample((element, None), &base, Some(!display_none));
        let (mut style, after_change) = changed_style(
            self.context.animations,
            (element, None),
            base,
            animating.as_ref(),
            parent,
            before,
            |style| !parent.unrendered && style.display != Display::None,
            |parent, animated, transitioned| {
                element_style(
                    self.document,
                    element,
                    &matched,
                    parent,
                    &resolution,
                    animated,
                    transitioned,
                )
            },
            // An animation of composited longhands alone, none of them
            // declared important, is the base style with its values set
            // (M35.2): what `compute_style` makes, the animations applied
            // after every normal declaration, nothing computed from them.
            // A revert keyword in a keyframe reads the user agent's
            // declarations, which only `compute_style` keeps (D73).
            |base, animated| {
                if !sets_composited
                    || important
                    || !animated.sets_only(&COMPOSITED_LONGHANDS)
                    || animated.reverts()
                {
                    return None;
                }
                let mut style = base.clone();
                let font = style.font.clone();
                let bases = LengthBases {
                    em: style.font_size_px,
                    font: font.as_deref().unwrap_or(crate::computed::initial_font()),
                    line: (
                        font.as_deref().unwrap_or(crate::computed::initial_font()),
                        style.font_size_px,
                    ),
                    resolution: &resolution,
                    color: style.color,
                    dark: style.dark,
                };
                animated.apply(
                    &mut style,
                    parent.style,
                    Target::Element,
                    &bases,
                    &mut [false; 4],
                    None,
                );
                #[cfg(test)]
                SET_ON.with(|set| set.set(set.get() + 1));
                Some(style)
            },
        );
        // Shown by a transition of `display` alone, it is inert (Display 4
        // §2.9): hit testing passes over it.
        if base_display == Display::None && style.display != Display::None {
            style.inert = true;
        }
        let unrendered = parent.unrendered || style.display == Display::None;
        let element_parent = Parent {
            style: &style,
            after_change: after_change.as_deref(),
            display_none,
            unrendered,
        };
        // Each pseudo-element some rule styles, from the element's style,
        // a list item's marker whether one does or not, as it is drawn
        // either way; with the animations and transitions of those that
        // run them, and those of the others cancelled.
        let pseudo = |target: Target| {
            let applied = matched.of_pseudo(target, element, &style, Some(&containers));
            let applied = &*applied;
            let styled = match target {
                Target::Marker => style.display.is_list_item(),
                // A `::backdrop` is an element's in the top layer alone
                // (CSS Position 4 section 4.1), which makes its box; one a
                // page styles is computed out of it too, for
                // `getComputedStyle`, as the reference browser computes it.
                Target::Backdrop => {
                    (!unrendered && self.document.in_top_layer(element))
                        || applied
                            .iter()
                            .any(|a| a.tier != tier(TierBase::UserAgent, false))
                }
                _ => !applied.is_empty(),
            };
            let pseudo = target.animated();
            if !styled {
                if let Some(pseudo) = pseudo {
                    self.sample((element, Some(pseudo)), &style, Some(false));
                    if let Some(cell) = self.context.animations {
                        cell.borrow_mut()
                            .cancel_transitions((element, Some(pseudo)));
                    }
                }
                return None;
            }
            let mut base = compute_style(applied, &style, target, &pseudo_resolution, None, None);
            if target == Target::Backdrop {
                top_layer::adjust_backdrop(&mut base);
            }
            let Some(pseudo) = pseudo else {
                return Some(base);
            };
            // A `::before` or an `::after` is rendered when it has a box.
            let has_box = |style: &ComputedStyle| {
                !unrendered
                    && style.display != Display::None
                    && !(target.takes_content() && style.content == Content::None)
            };
            let animating = self.sample(
                (element, Some(pseudo)),
                &base,
                Some(!display_none && has_box(&base)),
            );
            let before = match before {
                Before::Unknown => Before::Unknown,
                Before::Known(_) => Before::Known(
                    old.filter(|old| !old.unrendered)
                        .and_then(|old| old.pseudo.as_deref())
                        .and_then(|old| old.get(pseudo)),
                ),
            };
            let (style, _) = changed_style(
                self.context.animations,
                (element, Some(pseudo)),
                base,
                animating.as_ref(),
                element_parent,
                before,
                has_box,
                |parent, animated, transitioned| {
                    compute_style(
                        applied,
                        parent,
                        target,
                        &pseudo_resolution,
                        animated,
                        transitioned,
                    )
                },
                |_, _| None,
            );
            Some(style)
        };
        // A `details` element's `::details-content`, which the user
        // agent's sheet styles, and the `::before` and `::after` of it some
        // rule styles, from its style (CSS Pseudo-Elements 4 section 5.1).
        let details = |target: Target, parent: &ComputedStyle| {
            let applied = matched.of_pseudo(target, element, &style, Some(&containers));
            compute_style(&applied, parent, target, &pseudo_resolution, None, None)
        };
        // A closed one's skips its contents by `content-visibility:
        // hidden` (CSS Containment 2 section 4.1; gap batch H).
        let content = (self.document.tag(element) == Some("details")).then(|| {
            let mut content = details(Target::DetailsContent, &style);
            let skips = content.would_skip_contents(false);
            content.set_skips_contents(skips);
            content
        });
        let nested = |target: Target| {
            let content = content.as_ref()?;
            (!matched.of(target).is_empty()).then(|| details(target, content))
        };
        let details_content_before = nested(Target::DetailsContentBefore).map(Arc::new);
        let details_content_after = nested(Target::DetailsContentAfter).map(Arc::new);
        let pseudo = PseudoStyles {
            before: pseudo(Target::Before).map(Arc::new),
            after: pseudo(Target::After).map(Arc::new),
            first_letter: pseudo(Target::FirstLetter).map(Arc::new),
            first_line: pseudo(Target::FirstLine).map(Arc::new),
            marker: pseudo(Target::Marker).map(Arc::new),
            backdrop: pseudo(Target::Backdrop).map(Arc::new),
            details_content: content.map(Arc::new),
            details_content_before,
            details_content_after,
        };
        let pseudo = (pseudo != PseudoStyles::default()).then(|| Box::new(pseudo));
        // The kept base style itself when nothing changed it, so that the
        // elements made of the same share one (M35.9).
        let style = match shared {
            Some(shared) if *shared == style => shared,
            _ => Arc::new(style),
        };
        let reads_parent = matched
            .of(Target::Element)
            .iter()
            .any(|applied| crate::invalidation::inherits_explicitly(&applied.decl));
        Computed {
            style,
            pseudo,
            display_none,
            unrendered,
            after_change,
            reads_parent,
        }
    }

    /// What `element`'s base style is made of, if it may be kept for the
    /// elements after (M35.2): not when one of its declarations is its
    /// own (an inline style's, a presentational hint's) or reads an
    /// attribute (`attr()`), which another element's may not be.
    fn style_key(
        &mut self,
        element: NodeId,
        matched: &Matched<'_>,
        parent: &ComputedStyle,
        rem: f32,
    ) -> Option<StyleKey> {
        dom::hook!("style::style_key");
        if !self.keeps {
            return None;
        }
        // A MathML element's style is its name's and its place's as well
        // (D77): none is made of its declarations alone.
        if self.document.namespace(element) == Some(&dom::Namespace::MathMl) {
            return None;
        }
        // Nor is the style of an element in the top layer, of one with an
        // `inert` attribute or of the root while a modal dialog blocks the
        // document: their place makes it (gap batch H).
        if self.document.in_top_layer(element)
            || self.document.attr(element, "inert").is_some()
            || (self.document.document_element() == Some(element)
                && self.document.topmost_modal().is_some())
        {
            return None;
        }
        let mut declarations = Vec::new();
        for applied in matched.of(Target::Element) {
            let identity = applied.identity()?;
            let reads = *self
                .reads_attributes
                .entry(identity)
                .or_insert_with(|| custom::reads_attributes(&applied.decl.value));
            if reads {
                return None;
            }
            let Specificity(a, b, c) = applied.specificity;
            declarations.push((
                identity,
                applied.tier,
                applied.layer,
                [a, b, c, applied.context],
                applied.proximity,
                applied.order,
            ));
        }
        let parent_node = self.document.flat_parent(element)?;
        let parent = self.parent_number(parent_node, parent);
        Some(StyleKey {
            parent,
            root: self.document.document_element() == Some(element),
            unboxes: unboxes_to_nothing(self.document, element),
            relevant: relevant(self.document, element),
            dir: self.document.attr(element, "dir").map(Box::from),
            lang: self.document.attr(element, "lang").map(Box::from),
            rem: rem.to_bits(),
            declarations,
        })
    }

    /// The number of `node`'s style, `style`, among the distinct parent
    /// styles this cascade met: that of an equal one among the last
    /// [`RECENT_PARENTS`], else a new one.
    fn parent_number(&mut self, node: NodeId, style: &ComputedStyle) -> u32 {
        if let Some(&number) = self.parent_numbers.get(&node) {
            return number;
        }
        let number = match self.recent_parents.iter().find(|(_, seen)| seen == style) {
            Some(&(number, _)) => number,
            None => {
                let number = self.parent_numbers.len() as u32;
                if self.recent_parents.len() == RECENT_PARENTS {
                    self.recent_parents.pop_front();
                }
                self.recent_parents.push_back((number, style.clone()));
                number
            }
        };
        self.parent_numbers.insert(node, number);
        number
    }

    /// The `@keyframes` rule in force named `name` for an animation whose
    /// `animation-name` the tree of `tree` gave, `None` the document's:
    /// that tree's, else the tree of its host's, and so out to the
    /// document's (CSS Scoping 1 section 3.5, D69).
    fn keyframes_named(&self, tree: Option<NodeId>, name: &str) -> Option<KeyframesInForce<'a>> {
        dom::hook!("style::keyframes_named");
        let mut at = tree;
        let mut steps = 0;
        while let Some(root) = at.filter(|_| steps < dom::MAX_DEPTH) {
            if let Some(&found) = self.shadow_keyframes.get(&root).and_then(|k| k.get(name)) {
                return Some(found);
            }
            at = self
                .document
                .shadow_host(root)
                .map(|host| self.document.root_of(host))
                .filter(|&outer| self.document.is_shadow_root(outer));
            steps += 1;
        }
        self.keyframes.get(name).copied()
    }

    /// What `target`'s animations give at the timeline's time, its style
    /// without them being `base`: those in effect to sample, what those
    /// current or in effect animate, and the longhands they animate now
    /// and did in the target's last style. When `update` says whether the
    /// target is rendered, its animations are first matched to what
    /// `base` asks for, none when it is not (Animations 1 §2), and what
    /// they animate now recorded. `None` when it has none and had none,
    /// as nearly every target, which costs a test.
    fn sample(
        &self,
        target: crate::animation::Target,
        base: &ComputedStyle,
        update: Option<bool>,
    ) -> Option<Animating<'a>> {
        dom::hook!("style::sample");
        let cell = self.context.animations?;
        let mut before = Vec::new();
        if let Some(rendered) = update {
            let any = !self.keyframes.is_empty() || !self.shadow_keyframes.is_empty();
            let wanted = match (rendered, base.motion.as_deref()) {
                (true, Some(motion)) if any => wanted(motion, |name| {
                    self.keyframes_named(motion.animation_names_tree, name)
                }),
                _ => Vec::new(),
            };
            let mut animations = cell.borrow_mut();
            if wanted.is_empty()
                && !animations.animates(target)
                && animations.animated_longhands(target).is_empty()
            {
                return None;
            }
            before = animations.animated_longhands(target).to_vec();
            animations.update(target, &wanted);
        }
        let names_tree = base.motion.as_deref().and_then(|m| m.animation_names_tree);
        let mut animations = cell.borrow_mut();
        let mut samples = Vec::new();
        let mut acts = WillChange::AUTO;
        let mut now = Vec::new();
        for sample in animations.samples(target) {
            let keyframes = match sample.frames {
                SampleFrames::Css(name) => match self.keyframes_named(names_tree, &name) {
                    Some(keyframes) => animated::Frames::Sheet(keyframes),
                    None => continue,
                },
                SampleFrames::Script(keyframes) => animated::Frames::Script(keyframes),
            };
            let progress = sample.progress;
            let longhands = &keyframes.keyframes().longhands;
            if sample.current || progress.progress.is_some() {
                for longhand in longhands {
                    if let Some(feature) = WillChange::feature(longhand.name()) {
                        acts = acts.with(feature);
                    }
                }
            }
            if let Some(p) = progress.progress {
                now.extend_from_slice(longhands);
                samples.push(animated::Sampled {
                    keyframes,
                    composite: sample.composite,
                    progress: p,
                    before: progress.before,
                    easing: sample.easing,
                });
            }
        }
        now.sort_unstable();
        now.dedup();
        if update.is_some() {
            animations.set_animated_longhands(target, now.clone());
        }
        if samples.is_empty() && acts.is_auto() && before.is_empty() && now.is_empty() {
            return None;
        }
        let animated = (!samples.is_empty()).then_some(Animated { samples });
        Some(Animating {
            animated,
            acts,
            now,
            before,
        })
    }

    /// Every element's style, cascaded afresh; when `old` holds the
    /// styles the new ones replace, a change of style of every element
    /// from those (M22.1).
    fn cascade(&mut self, old: Option<&Styles>) -> Styles {
        let document = self.document;
        self.viewport_used.set(false);
        self.scheme_read.set(false);
        // The containers' sizes as the last layout laid them out (D72).
        if let Some(old) = old {
            self.container_sizes = old.container_sizes.clone();
        }
        self.queried.borrow_mut().clear();
        let mut styles = Styles {
            rem: ComputedStyle::default().font_size_px,
            media: self.media.clone(),
            scripting: self.context.media.scripting,
            page_scheme: self.page_scheme.clone(),
            ..Styles::default()
        };
        styles.set(document.root(), Computed::default());
        // `rem` is the root element's font size: known once the root (the
        // first element under the document) is styled, and the initial
        // value for the root itself.
        let mut root_font_size: Option<f32> = None;
        // Preorder with an explicit stack: an element's children are styled
        // together, from its own finished style, before any grandchild is
        // reached, and no depth of tree costs stack.
        let mut pending = vec![document.root()];
        while let Some(node) = pending.pop() {
            // What the children need of their parent, its pseudo-elements'
            // styles aside.
            let parent = styles
                .computed(node)
                .map(|p| Computed {
                    style: p.style.clone(),
                    pseudo: None,
                    display_none: p.display_none,
                    unrendered: p.unrendered,
                    after_change: p.after_change.clone(),
                    reads_parent: p.reads_parent,
                })
                .unwrap_or_default();
            // The flat tree's children (D68): a host's shadow tree's, a
            // slot's assigned elements, in their places.
            let children: Vec<NodeId> = document
                .flat_children(node)
                .filter(|&child| document.is_element(child))
                .collect();
            for &child in &children {
                let rem = root_font_size.unwrap_or(ComputedStyle::default().font_size_px);
                let replaced = old.and_then(|old| old.computed(child));
                // An element the styles replaced do not hold is one they
                // did not see, not one not rendered: what came before it
                // is unknown (D75), as a script's flush may have seen it.
                let before = match replaced {
                    Some(old) => Before::Known((!old.unrendered).then_some(&*old.style)),
                    None => Before::Unknown,
                };
                let computed =
                    self.style_of(child, Parent::of(&parent), rem, before, replaced, &styles);
                if root_font_size.is_none() {
                    root_font_size = Some(computed.style.font_size_px);
                    styles.rem = computed.style.font_size_px;
                    self.root_font = computed.style.font.clone();
                    styles.root_font = self.root_font.clone();
                }
                styles.set(child, computed);
            }
            pending.extend(children.into_iter().rev());
        }
        styles.uses_viewport_units = self.viewport_used.get();
        styles.uses_font_units = self.fonts_used.get();
        styles.container_sizes = self.container_sizes.clone();
        styles.queried = self.queried.take();
        styles.preferred_dark = self
            .scheme_read
            .get()
            .then_some(self.context.media.preferences.color_scheme == ColorScheme::Dark);
        // The animations and transitions of elements the cascade did not
        // reach are those of elements out of the document: cancelled.
        if let Some(animations) = self.context.animations {
            animations
                .borrow_mut()
                .retain(|node| styles.get(node).is_some());
        }
        styles
    }
}

/// The style of `target` after a change (M22.1, M22.2), from `base`,
/// its style without animations and transitions inheriting from
/// `parent`'s style, and `animating`, what its animations give: the
/// style with its animations' values, `compute` making it from the
/// parent's style the target inherits from, the animations and the
/// transitions; its after-change style, which inherits from the
/// parent's after-change style; its transitions started, replaced
/// and cancelled by Transitions 1 §3 when `before` is known; and the
/// style with the values of those running, but for a property an
/// animation animates. A target that `rendered` says is not rendered
/// by that style loses its transitions. The style, and its
/// after-change style when that differs.
#[allow(clippy::too_many_arguments)]
fn changed_style(
    animations: Option<&RefCell<Animations>>,
    target: crate::animation::Target,
    base: ComputedStyle,
    animating: Option<&Animating<'_>>,
    parent: Parent<'_>,
    before: Before<'_>,
    rendered: impl Fn(&ComputedStyle) -> bool,
    compute: impl Fn(&ComputedStyle, Option<&Animated<'_>>, Option<&[Transitioning]>) -> ComputedStyle,
    onto: impl Fn(&ComputedStyle, &Animated<'_>) -> Option<ComputedStyle>,
) -> (ComputedStyle, Option<Arc<ComputedStyle>>) {
    dom::hook!("style::changed_style");
    let acts = animating.map_or(WillChange::AUTO, |a| a.acts);
    let animated = animating.and_then(|a| a.animated.as_ref());
    let finish = |mut style: ComputedStyle| {
        if !acts.is_auto() {
            style.effects_mut().animated = acts;
        }
        style
    };
    let style = match animated {
        Some(animated) => finish(
            onto(&base, animated).unwrap_or_else(|| compute(parent.style, Some(animated), None)),
        ),
        None => finish(base),
    };
    let Some(cell) = animations else {
        return (style, None);
    };
    let after = parent
        .after_change
        .map(|parent| finish(compute(parent, animated, None)));
    if let Before::Known(before) = before {
        let skip = |longhand: Longhand| {
            animating.is_some_and(|a| a.now.contains(&longhand) || a.before.contains(&longhand))
        };
        cell.borrow_mut().update_transitions(
            target,
            before,
            after.as_ref().unwrap_or(&style),
            &skip,
        );
    }
    let transitioned: Vec<Transitioning> = {
        let animations = cell.borrow();
        if animations.transitions_of(target) {
            animations
                .transitioning(target)
                .into_iter()
                .filter(|t| match t.property {
                    Property::Longhand(longhand) => {
                        !animating.is_some_and(|a| a.now.contains(&longhand))
                    }
                    Property::Custom(_) => true,
                })
                .collect()
        } else {
            Vec::new()
        }
    };
    // A transition acts as `will-change` naming its property while it
    // runs (Web Animations 1 section 5.6, D76, G33), as an animation
    // does: a transform from `none` or an opacity from 1 makes the box
    // what it will be from the transition's start, and is composited.
    let transitions_act = transitioned
        .iter()
        .filter_map(|t| match &t.property {
            Property::Longhand(longhand) => WillChange::feature(longhand.name()),
            Property::Custom(_) => None,
        })
        .fold(WillChange::AUTO, WillChange::with);
    let mut transitioned_style = (!transitioned.is_empty()).then(|| {
        let mut style = finish(compute(parent.style, animated, Some(&transitioned)));
        if !transitions_act.is_auto() {
            let acting = style.effects().animated.with(transitions_act);
            style.effects_mut().animated = acting;
        }
        style
    });
    if !rendered(transitioned_style.as_ref().unwrap_or(&style)) {
        cell.borrow_mut().cancel_transitions(target);
        transitioned_style = None;
    }
    match transitioned_style {
        Some(shown) => (shown, Some(Arc::new(after.unwrap_or(style)))),
        None => {
            let after = after.filter(|after| *after != style).map(Arc::new);
            (style, after)
        }
    }
}

/// Whether `style` asks for a transition that runs: one of a combined
/// duration above zero (Transitions 1 section 3).
fn transitions_run(style: &ComputedStyle) -> bool {
    dom::hook!("style::transitions_run");
    style
        .motion()
        .transitions()
        .any(|t| t.duration.max(0.0) + t.delay > 0.0)
}

/// What one document's relative units resolve against, beyond the
/// element's own font size.
#[derive(Clone, Copy)]
struct Resolution<'a> {
    /// The root element's font size, for `rem`.
    rem: f32,
    /// Whether the style is the root element's (gap batch E).
    root: bool,
    /// The document's `color-scheme` meta element's value (D49).
    page_scheme: Option<&'a str>,
    context: &'a CascadeContext<'a>,
    /// Set when a viewport unit is met.
    viewport_used: &'a Cell<bool>,
    /// Set when a unit a font's figures give is met.
    fonts_used: Option<&'a Cell<bool>>,
    /// The root element's font, which the root units measure; `None`
    /// for the initial one, as on the root itself.
    root_font: Option<&'a FontProps>,
    /// The size the root units measure `root_font` at: the root's font
    /// size, `rem` but in the root's `line-height`.
    root_size: f32,
    /// Set when the user's colour scheme chooses the element's (D49).
    scheme_read: Option<&'a Cell<bool>>,
    /// The attributes of the element, or the originating element of the
    /// pseudo-element, the style is for: what `attr()` reads.
    attributes: &'a [(String, String)],
    /// What the substitutions of the cascade share (M21.1).
    substitutions: &'a Substitutions,
    /// What the container units read (D72), `None` where nothing is
    /// known of the document, and the element whose inclusive ancestors
    /// are the query containers: the element's parent, a pseudo-element's
    /// originating element.
    containers: Option<&'a Containers<'a>>,
    units_from: Option<NodeId>,
    /// Set when a container unit is met: the style is the element's own,
    /// not one another element of the same declarations may take.
    containers_used: &'a Cell<bool>,
}

impl Resolution<'_> {
    /// The width and height the container units measure (D72): the
    /// query containers' content boxes, the viewport's where there is
    /// none, which a resize computes again.
    fn container_units(&self) -> (f32, f32) {
        self.containers_used.set(true);
        self.viewport_used.set(true);
        match self.containers {
            Some(containers) => containers.unit_sizes(self.units_from),
            None => (self.context.viewport_width, self.context.viewport_height),
        }
    }
}

/// The bases a length in one element's declarations resolves against:
/// `em` is the element's own font size, except for `font-size` itself,
/// where it is the parent's.
#[derive(Clone, Copy)]
struct LengthBases<'a> {
    em: f32,
    /// The font `ex`, `ch`, `cap` and `ic` measure, at `em`: the
    /// element's, the parent's in the font properties.
    font: &'a FontProps,
    /// The font and size whose `line-height` `lh` is: the element's,
    /// the parent's in the font properties and `line-height`.
    line: (&'a FontProps, f32),
    resolution: &'a Resolution<'a>,
    /// The element's computed `color`, which `currentcolor` is in the
    /// other properties (gap batch E).
    color: Color,
    /// Whether the element uses the dark scheme (D49).
    dark: bool,
}

impl<'a> LengthBases<'a> {
    /// The bases of the font properties, `parent`'s: the font-relative
    /// units measure the parent's font (Values 4 section 6.1.1).
    fn of_parent(parent: &'a ComputedStyle, resolution: &'a Resolution<'a>) -> Self {
        LengthBases {
            em: parent.font_size_px,
            font: parent.font(),
            line: (parent.font(), parent.font_size_px),
            resolution,
            color: parent.color,
            dark: parent.dark,
        }
    }

    /// What the element's colours resolve against.
    fn colors(&self) -> Colors<'_> {
        Colors {
            current: self.color,
            dark: self.dark,
            lengths: Some(self),
        }
    }
}

/// A style rule in force, ready to match: its parsed selectors, its
/// declarations, and its cascade layer's place in the order of normal
/// declarations and of important ones (M20). A rule whose prelude does
/// not parse is dropped before this, per css-syntax-3 (M4.10).
pub(crate) struct ParsedRule<'s> {
    /// Its place among the rule set's rules: what describes it (M36).
    pub(crate) rule: u32,
    pub(crate) list: &'s SelectorList,
    /// The innermost `@scope` rule around it, by its place among the
    /// set's.
    pub(crate) scope: Option<u32>,
    /// The innermost `@container` block around it, by its place among
    /// the set's (D72).
    pub(crate) container: Option<u32>,
    pub(crate) declarations: &'s [css::Declaration],
    pub(crate) layer: u32,
    pub(crate) important_layer: u32,
    /// Whether every selector's subject is in SVG's namespace (the user
    /// agent's SVG rules, M32): no other element is tried against it.
    pub(crate) svg_only: bool,
}

/// The declarations that apply to an element and to each of its
/// pseudo-elements, unsorted, by [`Target`].
#[derive(Default)]
struct Matched<'a> {
    targets: [Vec<Applied<'a>>; 10],
}

impl<'a> Matched<'a> {
    fn of(&self, target: Target) -> &[Applied<'a>] {
        &self.targets[target as usize]
    }

    /// The declarations of the pseudo-element `target` of `element`,
    /// whose style is `own`: those of an `@container` block kept where
    /// it holds, the pseudo-element's query containers being the element
    /// and its ancestors (CSS Conditional 5 section 7.1, D72).
    fn of_pseudo(
        &self,
        target: Target,
        element: NodeId,
        own: &ComputedStyle,
        containers: Option<&Containers<'_>>,
    ) -> Cow<'_, [Applied<'a>]> {
        dom::hook!("style::of_pseudo");
        let applied = self.of(target);
        if applied.iter().all(|a| a.container().is_none()) {
            return Cow::Borrowed(applied);
        }
        let containers = containers.map(|c| Containers {
            own: Some((element, own)),
            ..*c
        });
        let mut held: HashMap<(usize, u32), bool> = HashMap::new();
        Cow::Owned(
            applied
                .iter()
                .filter(|a| match a.container() {
                    None => true,
                    Some((blocks, block)) => *held
                        .entry((blocks.as_ptr() as usize, block))
                        .or_insert_with(|| {
                            containers.as_ref().is_some_and(|containers| {
                                container_holds(blocks, block, Some(element), containers)
                            })
                        }),
                })
                .cloned()
                .collect(),
        )
    }

    /// The element's own declarations.
    fn element(&mut self) -> &mut Vec<Applied<'a>> {
        &mut self.targets[Target::Element as usize]
    }
}

/// A declaration plus everything needed to sort it into cascade order.
#[derive(Clone)]
struct Applied<'a> {
    /// See [`tier`] — encodes origin *and* importance in one order.
    tier: u8,
    /// The place of the rule's cascade layer among those of its origin
    /// and importance (Cascade 5 §6.4); zero for an inline style, which
    /// its tier puts above every layer.
    layer: u32,
    specificity: Specificity,
    /// How many generations up from the element its rule's scoping root
    /// is, [`UNSCOPED`] for a rule in no `@scope` (Cascade 6 section
    /// 6.5): after specificity, the nearer root wins.
    proximity: u32,
    /// Strictly increasing in the order this declaration was examined
    /// (UA sheet, then each author sheet, in document order) — the
    /// final tiebreak once tier and specificity are equal.
    order: u32,
    /// The `order` of its rule's first declaration: what `revert-rule`
    /// rolls back past (D73).
    rule: u32,
    /// The encapsulation context it comes from (Cascade 5 section 6.1,
    /// CSS Scoping 1 section 3.3, D69): how many shadow trees deep the
    /// tree of its rule is, the element's own for one not a rule's.
    context: u32,
    /// A stylesheet's declaration, borrowed, or an inline style's, parsed
    /// for the element.
    decl: Decl<'a>,
    /// What few declarations have besides (G247): kept apart, shared by a
    /// rule's, so that the many without are small to push and to sort.
    extra: Option<Arc<Extra<'a>>>,
}

/// A declaration an [`Applied`] holds: a stylesheet's, borrowed, or one
/// made for the element.
#[derive(Clone)]
enum Decl<'a> {
    Sheet(&'a css::Declaration),
    Own(Arc<css::Declaration>),
}

impl std::ops::Deref for Decl<'_> {
    type Target = css::Declaration;

    fn deref(&self) -> &css::Declaration {
        match self {
            Decl::Sheet(declaration) => declaration,
            Decl::Own(declaration) => declaration,
        }
    }
}

/// What an [`Applied`] has besides, when it has any of it.
struct Extra<'a> {
    /// The shadow root of its rule's tree, `None` for the document's:
    /// where a name it gives, as `animation-name` does, is looked up
    /// first (CSS Scoping 1 section 3.5).
    tree: Option<NodeId>,
    /// For a `::part()` rule's declaration, the host the element is a
    /// part of in the rule's tree: its originating element, whose
    /// attributes `attr()` reads (Values 5 section 7.7, as the reference
    /// browser has it).
    originating: Option<NodeId>,
    /// For a pseudo-element's declaration, the `@container` block of its
    /// sheet's blocks it is in, if any: whether it holds is known once
    /// the element's own style is, the element being the pseudo-element's
    /// first query container (D72).
    container: Option<(&'a [crate::rule_set::ContainerBlock], u32)>,
}

impl<'a> Extra<'a> {
    /// The extra of `tree`, `originating` and `container`, none when
    /// each is.
    fn of(
        tree: Option<NodeId>,
        originating: Option<NodeId>,
        container: Option<(&'a [crate::rule_set::ContainerBlock], u32)>,
    ) -> Option<Arc<Extra<'a>>> {
        (tree.is_some() || originating.is_some() || container.is_some()).then(|| {
            Arc::new(Extra {
                tree,
                originating,
                container,
            })
        })
    }
}

impl<'a> Applied<'a> {
    /// The shadow root of its rule's tree ([`Extra::tree`]).
    fn tree(&self) -> Option<NodeId> {
        self.extra.as_ref().and_then(|extra| extra.tree)
    }

    /// Its `::part()` rule's originating element ([`Extra::originating`]).
    fn originating(&self) -> Option<NodeId> {
        self.extra.as_ref().and_then(|extra| extra.originating)
    }

    /// Its `@container` block ([`Extra::container`]).
    fn container(&self) -> Option<(&'a [crate::rule_set::ContainerBlock], u32)> {
        self.extra.as_ref().and_then(|extra| extra.container)
    }

    /// The declaration's place in memory, when it is a stylesheet's,
    /// which every element it applies to shares and which stays put
    /// while the stylesheets live (M21.1); `None` for an inline style's.
    fn identity(&self) -> Option<usize> {
        match &self.decl {
            Decl::Sheet(declaration) => Some(std::ptr::from_ref(*declaration) as usize),
            Decl::Own(_) => None,
        }
    }
}

/// Where a declaration came from, before importance is folded in.
#[derive(Clone, Copy)]
enum TierBase {
    UserAgent,
    AuthorSheet,
    AuthorInline,
}

/// The cascade order (css-cascade-5 §6), collapsed to a single `u8` so
/// declarations can be sorted by `(tier, layer, specificity, order)` and
/// applied ascending — later always overwrites earlier, no special
/// cases at apply time. Lowest to highest precedence: UA normal, author
/// normal (inline beats any selector-matched normal rule, regardless of
/// specificity), author `!important` (inline again beats sheet rules),
/// UA `!important` (`assets/ua.css`'s hidden input and a form inside a
/// table's parts, which no author rule shows).
fn tier(base: TierBase, important: bool) -> u8 {
    match (base, important) {
        (TierBase::UserAgent, false) => 0,
        (TierBase::AuthorSheet, false) => 1,
        (TierBase::AuthorInline, false) => 2,
        (TierBase::AuthorSheet, true) => 3,
        (TierBase::AuthorInline, true) => 4,
        (TierBase::UserAgent, true) => 5,
    }
}

/// The rules the cascade matches an element against: the user agent's,
/// the document's, and each shadow tree's by its root (D69).
type RulesInForce<'r, 'a> = (
    &'r InForce<'a>,
    &'r InForce<'a>,
    &'r HashMap<NodeId, InForce<'a>>,
);

/// How many shadow trees deep the tree at `root` is: its encapsulation
/// context, as the cascade orders it (D69).
fn tree_depth(document: &Document, root: NodeId) -> u32 {
    dom::hook!("style::tree_depth");
    let mut depth = 0;
    let mut tree = root;
    while let Some(host) = document.shadow_host(tree) {
        depth += 1;
        tree = document.root_of(host);
    }
    depth
}

/// Every declaration that applies to `element` and to each of its
/// pseudo-elements, tagged with its cascade key — unsorted;
/// [`compute_style`] sorts before applying. Its own tree's rules apply
/// to it (a shadow tree's its own, D69); a host's shadow tree's rules by
/// `:host`; the rules of each slot's tree it is slotted into, through
/// slots slotted in turn, by `::slotted()`; and the rules of each tree
/// its part names reach, through the hosts' `exportparts`, by
/// `::part()`.
fn declarations_for<'a>(
    document: &Document,
    (matcher, candidates): (&mut Matcher<'a>, &mut Vec<(u32, u32)>),
    element: NodeId,
    (ua, author, shadow): RulesInForce<'_, 'a>,
    (inline_styles, top_layer): (Option<&dyn InlineStyles>, Option<&dyn TopLayer>),
    frame: FrameMargins,
    containers: Option<&Containers<'_>>,
) -> Matched<'a> {
    dom::hook!("style::declarations_for");
    let mut order = 0u32;
    let mut out = Matched::default();
    let svg = document.svg_tag(element).is_some();
    let own_tree = match document.has_shadow_roots() {
        true => document.root_of(element),
        false => document.root(),
    };
    let own = tree_depth(document, own_tree);
    let own_root = Some(own_tree).filter(|&root| document.is_shadow_root(root));
    matcher.set_shadow_scope(None);
    ua.candidates(document, element, candidates);
    collect_from_sheet(
        matcher,
        (element, svg),
        (ua, candidates),
        (TierBase::UserAgent, (own, own_root)),
        &mut order,
        &mut out,
        containers,
    );
    let ua_important = order;
    for decl in ua_important_declarations(document, element) {
        out.element().push(Applied {
            tier: tier(TierBase::UserAgent, decl.important),
            layer: 0,
            specificity: Specificity::default(),
            proximity: UNSCOPED,
            order,
            rule: ua_important,
            context: own,
            decl: Decl::Own(Arc::new(decl)),
            extra: Extra::of(own_root, None, None),
        });
        order += 1;
    }
    // HTML's presentational hints (M21.6): author declarations with no
    // specificity, before every author sheet's, so that any author rule
    // overrides them; together one rule.
    let hints = order;
    for decl in hint_declarations(document, element, frame) {
        out.element().push(Applied {
            tier: tier(TierBase::AuthorSheet, false),
            layer: 0,
            specificity: Specificity::default(),
            proximity: UNSCOPED,
            order,
            rule: hints,
            context: own,
            decl: Decl::Own(Arc::new(decl)),
            extra: Extra::of(own_root, None, None),
        });
        order += 1;
    }
    // What the top layer declares of it, an important declaration of the
    // user agent's (CSS Position 4 section 3.3, D76).
    if let Some(overlay) = top_layer.and_then(|layer| layer.overlay(element)) {
        let keyword = match overlay {
            Overlay::Auto => "auto",
            Overlay::None => "none",
        };
        out.element().push(Applied {
            tier: tier(TierBase::UserAgent, true),
            layer: 0,
            specificity: Specificity::default(),
            proximity: UNSCOPED,
            order,
            rule: order,
            context: own,
            decl: Decl::Own(Arc::new(css::Declaration {
                name: "overlay".to_owned(),
                value: vec![Token::Ident(keyword.into())],
                important: true,
            })),
            extra: Extra::of(own_root, None, None),
        });
        order += 1;
    }
    // Its own tree's rules: a shadow tree's, scoped to it.
    let own_rules = match document.is_shadow_root(own_tree) {
        true => {
            matcher.set_shadow_scope(Some(own_tree));
            shadow.get(&own_tree)
        }
        false => Some(author),
    };
    if let Some(rules) = own_rules {
        rules.candidates(document, element, candidates);
        collect_from_sheet(
            matcher,
            (element, svg),
            (rules, candidates),
            (TierBase::AuthorSheet, (own, own_root)),
            &mut order,
            &mut out,
            containers,
        );
    }
    // The trees around and below it: `::part()` reaches it from the
    // document's rules though no shadow tree has a sheet of its own.
    if document.has_shadow_roots() {
        scoped_declarations(
            document,
            (matcher, candidates),
            (element, svg),
            (author, shadow),
            (&mut order, &mut out),
            containers,
            None,
        );
    }
    matcher.set_shadow_scope(None);
    // An inline `style` styles the element alone, if its document's
    // policies allow it (M33.1).
    if let Some(style_attr) = document.attr(element, "style")
        && inline_styles.is_none_or(|inline| inline.allowed(element, style_attr))
        && let Ok(decls) = css::parse_style_attribute(style_attr)
    {
        // The attribute's declarations are one rule.
        let rule = order;
        for decl in decls {
            out.element().push(Applied {
                tier: tier(TierBase::AuthorInline, decl.important),
                layer: 0,
                // Inline declarations aren't selector-matched, so there's
                // nothing to compare within this tier by specificity —
                // `order` alone (declaration order in the attribute)
                // breaks ties, matching real engines.
                specificity: Specificity::default(),
                proximity: UNSCOPED,
                order,
                rule,
                context: own,
                decl: Decl::Own(Arc::new(decl)),
                extra: Extra::of(own_root, None, None),
            });
            order += 1;
        }
    }
    out
}

/// The declarations other trees' rules give `element` (CSS Scoping 1,
/// CSS Shadow Parts 1, D69): its shadow tree's by `:host`, its slots'
/// trees' by `::slotted()`, and the trees its parts reach by `::part()`,
/// each in its tree's context; and its pseudo-elements' by those
/// followed by one (`::part(x)::before`). With `highlight`, those of its
/// `::highlight()` of that name alone, in the element's place.
fn scoped_declarations<'a>(
    document: &Document,
    (matcher, candidates): (&mut Matcher<'a>, &mut Vec<(u32, u32)>),
    (element, svg): (NodeId, bool),
    (author, shadow): (&InForce<'a>, &HashMap<NodeId, InForce<'a>>),
    (order, out): (&mut u32, &mut Matched<'a>),
    containers: Option<&Containers<'_>>,
    highlight: Option<HighlightName>,
) {
    dom::hook!("style::scoped_declarations");
    // A host: its shadow tree's rules, which match it featureless, by
    // `:host` alone, and its pseudo-elements by `:host::before`.
    if let Some(root) = document.shadow_root(element)
        && let Some(rules) = shadow.get(&root)
    {
        matcher.set_shadow_scope(Some(root));
        rules.candidates(document, element, candidates);
        collect_from_sheet_for(
            matcher,
            (element, svg),
            (rules, candidates),
            (
                TierBase::AuthorSheet,
                (tree_depth(document, root), Some(root)),
            ),
            (order, out),
            containers,
            highlight,
        );
    }
    // Slotted: the rules of each slot's tree, the slot itself slotted
    // in turn into the next, by `::slotted()` of the slot.
    let mut slot = document.assigned_slot(element);
    let mut steps = 0;
    while let Some(at) = slot {
        let root = document.root_of(at);
        if let Some(rules) = shadow.get(&root) {
            matcher.set_shadow_scope(Some(root));
            rules.candidates(document, at, candidates);
            collect_scoped(
                (rules, candidates),
                (
                    TierBase::AuthorSheet,
                    (tree_depth(document, root), Some(root)),
                ),
                (element, containers, None),
                order,
                out,
                (document, highlight),
                |selector| matcher.matches_slotted(element, at, selector),
            );
        }
        steps += 1;
        slot = document
            .assigned_slot(at)
            .filter(|_| steps < dom::MAX_DEPTH);
    }
    // Parts: the names `part` gives, as each host's tree sees them
    // through its `exportparts`, matched by `::part()` of the host.
    let mut names: Vec<String> = document
        .attr(element, "part")
        .map(|value| value.split_ascii_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let own = document.root_of(element);
    // Its own tree's rules reach it through its host, `:host::part()`
    // (CSS Shadow Parts 1 section 4.2), by the names it gives itself.
    if let Some(at) = document.shadow_host(own).filter(|_| !names.is_empty())
        && let Some(rules) = shadow.get(&own)
    {
        matcher.set_shadow_scope(Some(own));
        let seen: Vec<&str> = names.iter().map(String::as_str).collect();
        rules.candidates(document, at, candidates);
        collect_scoped(
            (rules, candidates),
            (
                TierBase::AuthorSheet,
                (tree_depth(document, own), Some(own)),
            ),
            (element, containers, Some(at)),
            order,
            out,
            (document, highlight),
            |selector| matcher.matches_part(element, at, &seen, selector),
        );
    }
    let mut host = document.shadow_host(own);
    while let Some(at) = host.filter(|_| !names.is_empty()) {
        let tree = document.root_of(at);
        let rules = match document.is_shadow_root(tree) {
            true => {
                matcher.set_shadow_scope(Some(tree));
                shadow.get(&tree)
            }
            false => {
                matcher.set_shadow_scope(None);
                Some(author)
            }
        };
        if let Some(rules) = rules {
            let seen: Vec<&str> = names.iter().map(String::as_str).collect();
            rules.candidates(document, at, candidates);
            collect_scoped(
                (rules, candidates),
                (
                    TierBase::AuthorSheet,
                    (
                        tree_depth(document, tree),
                        Some(tree).filter(|&tree| document.is_shadow_root(tree)),
                    ),
                ),
                (element, containers, Some(at)),
                order,
                out,
                (document, highlight),
                |selector| matcher.matches_part(element, at, &seen, selector),
            );
        }
        names = exported_parts(document.attr(at, "exportparts"), &names);
        host = document.shadow_host(tree);
    }
}

/// The part names a host's `exportparts` (CSS Shadow Parts 1 section
/// 4.1) makes of `names`, those of an element of its shadow tree: each
/// entry `inner` or `inner: outer` whose inner name is among them gives
/// its outer one.
fn exported_parts(exportparts: Option<&str>, names: &[String]) -> Vec<String> {
    dom::hook!("style::exported_parts");
    let Some(value) = exportparts else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in value.split(',') {
        let (inner, outer) = match entry.split_once(':') {
            Some((inner, outer)) => (inner.trim(), outer.trim()),
            None => (entry.trim(), entry.trim()),
        };
        let valid = |name: &str| !name.is_empty() && !name.contains(char::is_whitespace);
        if valid(inner) && valid(outer) && names.iter().any(|name| name == inner) {
            out.push(outer.to_owned());
        }
    }
    out
}

/// Pushes the declarations of `sheet`'s rules one of whose `candidates`
/// ending in `::slotted()` or `::part()` `matches`, for the element, or
/// for its pseudo-element the selector names after them
/// (`::part(x)::before`), with the highest specificity among those of a
/// rule that match for each, `base`'s tier and the context given; for
/// `::part()`, the host `originating` it is a part of. With
/// `highlight`, those followed by its `::highlight()` of that name
/// alone, in the element's place.
fn collect_scoped<'a>(
    (sheet, candidates): (&InForce<'a>, &[(u32, u32)]),
    (base, (context, tree)): (TierBase, (u32, Option<NodeId>)),
    (element, containers, originating): (NodeId, Option<&Containers<'_>>, Option<NodeId>),
    order: &mut u32,
    out: &mut Matched<'a>,
    (document, highlight): (&Document, Option<HighlightName>),
    mut matches: impl FnMut(&'a Selector) -> bool,
) {
    dom::hook!("style::collect_scoped");
    let parent = containers.and_then(|containers| containers.document.flat_parent_element(element));
    let details = document.tag(element) == Some("details");
    let mut next = 0;
    while next < candidates.len() {
        let place = candidates[next].0;
        let start = next;
        while next < candidates.len() && candidates[next].0 == place {
            next += 1;
        }
        let rule = &sheet[place as usize];
        let list: &'a SelectorList = rule.list;
        let mut best: [Option<Specificity>; Target::ALL.len()] = [None; Target::ALL.len()];
        let mut hit = 0u16;
        for &(_, index) in &candidates[start..next] {
            let selector = &list.0[index as usize];
            let Some(ending) = selector.pseudo_element.as_ref().filter(|ending| {
                matches!(ending.element, PseudoElement::Slotted | PseudoElement::Part)
            }) else {
                continue;
            };
            let target = match (ending.then.as_deref(), highlight) {
                (None, None) => Target::Element,
                (Some(then), None) if then.then.is_none() => {
                    let of_details = matches!(
                        then.element,
                        PseudoElement::DetailsContent
                            | PseudoElement::DetailsContentBefore
                            | PseudoElement::DetailsContentAfter
                    );
                    match Target::of(then.element) {
                        Some(target) if details || !of_details => target,
                        // One matched and set aside: nothing draws it yet.
                        _ => continue,
                    }
                }
                // The highlight asked for, in the element's place.
                (Some(then), Some(name))
                    if then.then.is_none() && then.argument == PseudoArgument::Highlight(name) =>
                {
                    Target::Element
                }
                _ => continue,
            };
            if matches(selector) {
                let at = target as usize;
                hit |= 1 << at;
                best[at] = best[at].max(Some(selector.specificity));
            }
        }
        if hit == 0 {
            continue;
        }
        for target in Target::ALL {
            if hit & 1 << target as u16 == 0 {
                continue;
            }
            let Some(specificity) = best[target as usize] else {
                continue;
            };
            // In a `@container` block, the element's query containers
            // are its flat tree ancestors'; a pseudo-element's block is
            // weighed once the element's style is known
            // ([`Matched::of_pseudo`]), its own and theirs (D72).
            let mut container = None;
            if let Some(block) = rule.container {
                if target == Target::Element {
                    let holds = containers.is_some_and(|containers| {
                        container_holds(sheet.containers, block, parent, containers)
                    });
                    if !holds {
                        continue;
                    }
                } else {
                    container = Some((sheet.containers, block));
                }
            }
            let extra = Extra::of(tree, originating, container);
            for (offset, decl) in rule.declarations.iter().enumerate() {
                out.targets[target as usize].push(Applied {
                    tier: tier(base, decl.important),
                    layer: if decl.important {
                        rule.important_layer
                    } else {
                        rule.layer
                    },
                    specificity,
                    proximity: UNSCOPED,
                    order: *order + offset as u32,
                    rule: *order,
                    context,
                    decl: Decl::Sheet(decl),
                    extra: extra.clone(),
                });
            }
        }
        *order += rule.declarations.len() as u32;
    }
}

/// The declarations of the rules that style `element`'s `::highlight()`
/// of the name `name` (gap batch H), in the element's place of the
/// [`Matched`] made: a sheet's rules alone, no presentational hint nor
/// inline style styling a pseudo-element.
fn highlight_declarations<'a>(
    document: &Document,
    (matcher, candidates): (&mut Matcher<'a>, &mut Vec<(u32, u32)>),
    element: NodeId,
    (ua, author, shadow): RulesInForce<'_, 'a>,
    name: HighlightName,
    containers: Option<&Containers<'_>>,
) -> Matched<'a> {
    dom::hook!("style::highlight_declarations");
    let mut order = 0u32;
    let mut out = Matched::default();
    let svg = document.svg_tag(element).is_some();
    // The user agent's rules, then its own tree's, a shadow tree's scoped
    // to it, as `declarations_for` takes them.
    let own_tree = match document.has_shadow_roots() {
        true => document.root_of(element),
        false => document.root(),
    };
    let own = tree_depth(document, own_tree);
    let own_root = Some(own_tree).filter(|&root| document.is_shadow_root(root));
    matcher.set_shadow_scope(None);
    ua.candidates(document, element, candidates);
    collect_from_sheet_for(
        matcher,
        (element, svg),
        (ua, candidates),
        (TierBase::UserAgent, (own, own_root)),
        (&mut order, &mut out),
        containers,
        Some(name),
    );
    let own_rules = match document.is_shadow_root(own_tree) {
        true => {
            matcher.set_shadow_scope(Some(own_tree));
            shadow.get(&own_tree)
        }
        false => Some(author),
    };
    if let Some(rules) = own_rules {
        rules.candidates(document, element, candidates);
        collect_from_sheet_for(
            matcher,
            (element, svg),
            (rules, candidates),
            (TierBase::AuthorSheet, (own, own_root)),
            (&mut order, &mut out),
            containers,
            Some(name),
        );
    }
    // The trees around and below it, by `::part()::highlight()` and
    // `::slotted()::highlight()` (gap batch H).
    if document.has_shadow_roots() {
        scoped_declarations(
            document,
            (matcher, candidates),
            (element, svg),
            (author, shadow),
            (&mut order, &mut out),
            containers,
            Some(name),
        );
    }
    matcher.set_shadow_scope(None);
    out
}

/// `element`'s presentational hints (M21.6) and, for an SVG element,
/// its presentation attributes (M32), built as declarations, never as
/// text a value could add another to.
fn hint_declarations(
    document: &Document,
    element: NodeId,
    frame: FrameMargins,
) -> Vec<css::Declaration> {
    let hints = document.tag(element).map_or_else(String::new, |tag| {
        let attr = |name: &str| document.attr(element, name).map(str::to_string);
        lists::list_hints(tag, attr)
            + &images::img_hints(tag, attr)
            + &images::frame_hints(tag, attr)
            + &presentational::hints(document, element, tag, frame)
    });
    let svg_hints = match (document.svg_tag(element), document.attrs(element)) {
        (Some(tag), Some(attrs)) => svg::svg_hints(tag, attrs),
        _ => Vec::new(),
    };
    let mut declarations = if hints.is_empty() {
        Vec::new()
    } else {
        css::parse_style_attribute(&hints).unwrap_or_default()
    };
    declarations.extend(svg_hints);
    declarations
}

/// The user agent's `!important` declarations for `element` that no
/// selector can say, as the reference browser's sheet says them for an
/// HTML document alone: a `form` the HTML parser left a child of a
/// table, a row group or a row is not shown (HTML section 15.3.8), and
/// one in an XML document is.
fn ua_important_declarations(document: &Document, element: NodeId) -> Vec<css::Declaration> {
    dom::hook!("style::ua_important_declarations");
    let in_table = document.tag(element) == Some("form")
        && document
            .parent(element)
            .and_then(|p| document.tag(p))
            .is_some_and(|p| matches!(p, "table" | "thead" | "tbody" | "tfoot" | "tr"))
        && !document.is_xml_node(element);
    if !in_table {
        return Vec::new();
    }
    css::parse_style_attribute("display: none !important").unwrap_or_default()
}

/// Matches `sheet`'s rules against `element` and its pseudo-elements,
/// pushing one [`Applied`] per declaration of every rule that matches,
/// for each target it matches. Only `candidates` are tried, the
/// selectors the rule index gives for the element (M35.2), by their
/// rules' places and their own, in order: a selector of no candidate
/// cannot match the element.
fn collect_from_sheet<'a>(
    matcher: &mut Matcher<'a>,
    (element, svg): (NodeId, bool),
    (sheet, candidates): (&InForce<'a>, &[(u32, u32)]),
    (base, (context, tree)): (TierBase, (u32, Option<NodeId>)),
    order: &mut u32,
    out: &mut Matched<'a>,
    containers: Option<&Containers<'_>>,
) {
    dom::hook!("style::collect_from_sheet");
    collect_from_sheet_for(
        matcher,
        (element, svg),
        (sheet, candidates),
        (base, (context, tree)),
        (order, out),
        containers,
        None,
    );
}

/// [`collect_from_sheet`], or with `highlight` the rules for the
/// element's `::highlight()` of that name alone, their declarations in
/// the element's place.
fn collect_from_sheet_for<'a>(
    matcher: &mut Matcher<'a>,
    (element, svg): (NodeId, bool),
    (sheet, candidates): (&InForce<'a>, &[(u32, u32)]),
    (base, (context, tree)): (TierBase, (u32, Option<NodeId>)),
    (order, out): (&mut u32, &mut Matched<'a>),
    containers: Option<&Containers<'_>>,
    highlight: Option<HighlightName>,
) {
    dom::hook!("style::collect_from_sheet_for");
    // The scoping roots of each scope met, for `element`.
    let mut roots: HashMap<u32, Vec<(NodeId, u32)>> = HashMap::new();
    // Whether each `@container` block met holds for the element, whose
    // query containers are its ancestors (D72).
    let mut held: HashMap<u32, bool> = HashMap::new();
    let parent = matcher.document().flat_parent_element(element);
    let mut next = 0;
    while next < candidates.len() {
        let place = candidates[next].0;
        let start = next;
        while next < candidates.len() && candidates[next].0 == place {
            next += 1;
        }
        let rule = &sheet[place as usize];
        if rule.svg_only && !svg {
            continue;
        }
        let scoped = rule.scope.map(|scope| {
            roots
                .entry(scope)
                .or_insert_with(|| scoping_roots(matcher, sheet, scope, element))
                .as_slice()
        });
        if scoped.is_some_and(<[_]>::is_empty) {
            continue;
        }
        // A comma-separated list is cascade-equivalent to separate rules
        // sharing one declaration block — each matching branch
        // contributes independently, so the specificity that matters is
        // the highest among the branches that actually matched here,
        // for the element and for each of its pseudo-elements, and of
        // those the nearest scoping root (Cascade 6 section 6.5).
        let list: &'a SelectorList = rule.list;
        let mut best: [Option<(Specificity, Reverse<u32>)>; 10] = [None; 10];
        // The targets matched, a bit each (G247): most rules match none,
        // and the rest one.
        let mut hit = 0u16;
        for &(_, index) in &candidates[start..next] {
            let selector = &list.0[index as usize];
            let target = match (&selector.pseudo_element, highlight) {
                (None, None) => Target::Element,
                // One matched and set aside: nothing draws it yet.
                (Some(ending), None) => match Target::of(ending.element) {
                    Some(target) => target,
                    None => continue,
                },
                // The highlight asked for, in the element's place.
                (Some(ending), Some(name))
                    if ending.argument == PseudoArgument::Highlight(name) =>
                {
                    Target::Element
                }
                (_, Some(_)) => continue,
            };
            let matches = |matcher: &mut Matcher<'a>| match &selector.pseudo_element {
                None => matcher.matches_element(element, selector),
                Some(ending) => matcher.matches_pseudo_element(element, selector, ending.element),
            };
            let proximity = match scoped {
                None => matches(matcher).then_some(UNSCOPED),
                Some(roots) => {
                    let nearest = roots.iter().find(|&&(root, _)| {
                        matcher.set_rule_scope(Some(root));
                        matches(matcher)
                    });
                    matcher.set_rule_scope(None);
                    nearest.map(|&(_, proximity)| proximity)
                }
            };
            if let Some(proximity) = proximity {
                hit |= 1 << target as u16;
                let best = &mut best[target as usize];
                *best = (*best).max(Some((selector.specificity, Reverse(proximity))));
            }
        }
        if hit == 0 {
            continue;
        }
        for target in Target::ALL {
            if hit & 1 << target as u16 == 0 {
                continue;
            }
            let Some((specificity, Reverse(proximity))) = best[target as usize] else {
                continue;
            };
            // A pseudo-element's block is weighed once the element's
            // style is known ([`Matched::of_pseudo`]).
            let pseudo = target != Target::Element;
            if let Some(block) = rule.container
                && !pseudo
            {
                let holds = *held.entry(block).or_insert_with(|| {
                    containers.is_some_and(|containers| {
                        container_holds(sheet.containers, block, parent, containers)
                    })
                });
                if !holds {
                    continue;
                }
            }
            let container = rule
                .container
                .filter(|_| pseudo)
                .map(|block| (sheet.containers, block));
            // The same order for each target: within one, it is the
            // declarations' order in the sheets.
            let extra = Extra::of(tree, None, container);
            for (offset, decl) in rule.declarations.iter().enumerate() {
                out.targets[target as usize].push(Applied {
                    tier: tier(base, decl.important),
                    layer: if decl.important {
                        rule.important_layer
                    } else {
                        rule.layer
                    },
                    specificity,
                    proximity,
                    order: *order + offset as u32,
                    rule: *order,
                    context,
                    decl: Decl::Sheet(decl),
                    extra: extra.clone(),
                });
            }
        }
        *order += rule.declarations.len() as u32;
    }
}

/// Whether `block`, one of `blocks`, holds for an element whose query
/// containers are `from` and its ancestors: its queries and those of
/// every block around it (D72).
fn container_holds(
    blocks: &[crate::rule_set::ContainerBlock],
    block: u32,
    from: Option<NodeId>,
    containers: &Containers<'_>,
) -> bool {
    dom::hook!("style::container_holds");
    let mut at = Some(block);
    while let Some(index) = at {
        let block = &blocks[index as usize];
        if !block.rule.holds(from, containers) {
            return false;
        }
        at = block.parent;
    }
    true
}

/// The scope proximity of a declaration of no `@scope` rule: farther
/// than any (Cascade 6 section 6.5).
const UNSCOPED: u32 = u32::MAX;

/// The scoping roots of `scope`, one of `sheet`'s `@scope` rules, that
/// `element` is in the scope of, nearest first, each with how many
/// generations up from `element` it is (Cascade 6 section 2.5): its
/// inclusive ancestors that match the rule's root selectors, `:scope` in
/// them a root of the scope around it that they are in the scope of,
/// with no limit between them and `element`, `element` itself included.
/// An implicit root is its sheet's, the parent element of the node that
/// owns it, or the root element for a sheet with none.
fn scoping_roots<'a>(
    matcher: &mut Matcher<'a>,
    sheet: &InForce<'a>,
    scope: u32,
    element: NodeId,
) -> Vec<(NodeId, u32)> {
    dom::hook!("style::scoping_roots");
    let document = matcher.document();
    let rule = &sheet.scopes[scope as usize];
    let outer = rule
        .parent
        .map(|parent| scoping_roots(matcher, sheet, parent, element));
    // A shadow tree's sheet with no element around its owner, or one it
    // adopted, scopes to its host (D69).
    let host = matcher.scoped_host();
    let implicit = || {
        sheet
            .roots
            .get(rule.sheet as usize)
            .copied()
            .flatten()
            .or(host)
            .or_else(|| document.document_element())
    };
    let implicit = if rule.start.is_none() {
        implicit()
    } else {
        None
    };
    let mut roots = Vec::new();
    let mut at = Some(element);
    let mut generations = 0u32;
    while let Some(node) = at {
        let is_root = match (&rule.start, &outer) {
            (None, _) => implicit == Some(node),
            (Some(start), None) => {
                matcher.set_rule_scope(None);
                matcher.matches_list(node, start)
            }
            // The root of an inner scope is in the scope of an outer root,
            // which `:scope` in its selectors is.
            (Some(start), Some(outer)) => outer.iter().any(|&(root, up)| {
                up >= generations && {
                    matcher.set_rule_scope(Some(root));
                    let found = matcher.matches_list(node, start);
                    matcher.set_rule_scope(None);
                    found
                }
            }),
        };
        let inside_outer = outer
            .as_ref()
            .is_none_or(|outer| outer.iter().any(|&(_, up)| up >= generations));
        if is_root && inside_outer && !limited(matcher, rule.end.as_ref(), node, element) {
            roots.push((node, generations));
        }
        // A shadow tree's rules reach up to its host, and no further.
        if Some(node) == host {
            break;
        }
        at = document.parent_element(node).or_else(|| {
            document
                .parent(node)
                .filter(|&parent| document.is_shadow_root(parent))
                .and(host)
        });
        generations += 1;
    }
    matcher.set_rule_scope(None);
    roots
}

/// Whether a limit of the scope of `root` stands between it and
/// `element`: an element from `element` up to the root, the root too,
/// that the limit selectors `end` match, `:scope` in them the root. A
/// limit selector that names no `:scope` is relative to it, and so never
/// the root; one that does, `to (:scope)`, may leave nothing in scope.
fn limited<'a>(
    matcher: &mut Matcher<'a>,
    end: Option<&'a SelectorList>,
    root: NodeId,
    element: NodeId,
) -> bool {
    dom::hook!("style::limited");
    let Some(end) = end else {
        return false;
    };
    let document = matcher.document();
    matcher.set_rule_scope(Some(root));
    let mut at = Some(element);
    let mut found = false;
    while let Some(node) = at {
        if matcher.matches_list(node, end) {
            found = true;
            break;
        }
        if node == root {
            break;
        }
        at = document.parent_element(node);
    }
    matcher.set_rule_scope(None);
    found
}

/// Sorts `applied` into cascade order and folds it into a fresh
/// `ComputedStyle` inherited from `parent`, for `target`, in three passes
/// (M21): the custom properties, which the other values substitute; then
/// `font-size`, which the other lengths' `em` is of; then the rest. The
/// values of the target's animations in effect, `animated`, apply in each
/// pass after the normal declarations and before the important ones, the
/// animations' origin, and those of its running transitions,
/// `transitioned`, after everything, the transitions' (Cascade 5 §6.1,
/// M22.1, M22.2).
fn compute_style(
    applied: &[Applied<'_>],
    parent: &ComputedStyle,
    target: Target,
    resolution: &Resolution<'_>,
    animated: Option<&Animated<'_>>,
    transitioned: Option<&[Transitioning]>,
) -> ComputedStyle {
    dom::hook!("style::compute_style");
    let transitioned = transitioned.unwrap_or_default();
    // `overlay` is the user agent's alone (CSS Position 4 section 3.2):
    // an author's declaration of it, important or not, is valid but
    // ignored, as the reference browser ignores it; an author only
    // transitions the value the user agent's sheet gives (D76).
    let mut sorted: Vec<&Applied<'_>> = applied
        .iter()
        .filter(|a| {
            a.decl.name != "overlay"
                || a.tier == tier(TierBase::UserAgent, false)
                || a.tier == tier(TierBase::UserAgent, true)
        })
        .collect();
    // Origin and importance, then the context (Cascade 5 section 6.1):
    // for normal declarations the outer tree's win, for important ones
    // the inner's; then whether the element's own, the layer, the
    // specificity, the scope proximity and the order.
    sorted.sort_by_key(|a| {
        let (rank, context) = match a.tier {
            0 => (0, 0),
            1 | 2 => (1, u32::MAX - a.context),
            3 | 4 => (2, a.context),
            _ => (3, 0),
        };
        (
            rank,
            context,
            a.tier,
            a.layer,
            a.specificity,
            Reverse(a.proximity),
            a.order,
        )
    });
    // Where the important declarations begin.
    let important = sorted.partition_point(|a| a.tier < tier(TierBase::AuthorSheet, true));
    // Where the revert keywords roll back to, which each pass keeps its
    // state at (D73).
    let rollbacks = Rollbacks::of(&sorted, important, animated.is_some_and(Animated::reverts));

    let mut style = ComputedStyle::inherited_from(parent);

    // The custom properties first (M21.1), on every target: the other
    // values substitute them. An element that declares none keeps its
    // parent's, which it inherited. Each is kept with its place, the
    // animations' at the important declarations' start, for the revert
    // keywords, which take away the declarations of their name from
    // where they roll back to.
    type Placed<'d> = ((usize, bool), custom::Declaration<'d>);
    fn custom_of<'d>(applied: &[&'d Applied<'_>], first: usize) -> Vec<Placed<'d>> {
        dom::hook!("style::custom_of");
        applied
            .iter()
            .enumerate()
            .filter(|(_, a)| custom::is_custom_property_name(&a.decl.name))
            .map(|(n, a)| {
                let declaration = custom::Declaration {
                    name: &a.decl.name,
                    value: &a.decl.value,
                    identity: a.identity(),
                    animated: false,
                    attr: false,
                };
                ((first + n, false), declaration)
            })
            .collect()
    }
    // The names registered with a syntax, whose animations' values are
    // interpolated by it once the declared ones are computed (D73).
    let typed = |name: &str| {
        resolution
            .substitutions
            .registry
            .get(name)
            .is_some_and(|r| !r.syntax.is_universal())
    };
    let mut placed = custom_of(&sorted[..important], 0);
    if let Some(animated) = animated {
        let animations = animated.custom_declarations(&typed);
        placed.extend(animations.into_iter().map(|d| ((important, true), d)));
    }
    placed.extend(custom_of(&sorted[important..], important));
    for (n, a) in sorted.iter().enumerate().filter(|_| !rollbacks.is_empty()) {
        let Some(keyword) = revert_keyword(&a.decl.value) else {
            continue;
        };
        if !custom::is_custom_property_name(&a.decl.name) {
            continue;
        }
        let mark = rollbacks.mark(&sorted, n, keyword);
        placed.retain(|&(place, ref declaration)| {
            *declaration.name != *a.decl.name
                || rollbacks.counts_before(mark, place)
                || (if place.1 { important } else { place.0 }) > n
        });
    }
    let mut declared: Vec<custom::Declaration<'_>> = placed.into_iter().map(|(_, d)| d).collect();
    // A transition of a custom property gives its value at the step, or
    // the guaranteed-invalid value, which `initial` is. It reads an
    // attribute as the value it steps to did, or, interpolated by its
    // type below, as either did (Values 5 section 7.7.3).
    let initial = [Token::Ident("initial".into())];
    for transition in transitioned {
        if let (
            Property::Custom(name),
            Value::Custom(start, start_attr),
            Value::Custom(end, end_attr),
        ) = (&transition.property, &transition.start, &transition.end)
        {
            let (value, attr) = if transition.progress < 0.5 {
                (start, *start_attr)
            } else {
                (end, *end_attr)
            };
            let interpolated = || match (start, end) {
                (Some(start), Some(end)) => {
                    typed(name)
                        && crate::property::interpolate(start, end, transition.progress).is_some()
                }
                _ => false,
            };
            declared.push(custom::Declaration {
                name,
                value: value.as_deref().unwrap_or(&initial),
                identity: None,
                animated: false,
                attr: attr || ((*start_attr || *end_attr) && interpolated()),
            });
        }
    }
    // A registered property takes its initial value where nothing
    // declares it on the root, and where it is not inherited (D73).
    if !declared.is_empty() || !resolution.substitutions.registry.is_empty() {
        style.custom = resolution.substitutions.compute(
            &declared,
            parent.custom.as_ref(),
            resolution.attributes,
        );
    }

    // `font-size` next: every other length's `em` resolves against
    // *this* element's own font-size, which itself must already be
    // final before those are processed. Its own `em` (and `%`) is the
    // parent's font size.
    let mut sizes: Vec<(Mark, f32)> = Vec::new();
    // The registered custom properties in a cycle with the font or the
    // line, and the declarations that read them, by their places (D73).
    let mut cyclic: Vec<&str> = Vec::new();
    let mut invalid: Vec<usize> = Vec::new();
    for (n, a) in sorted.iter().enumerate() {
        rollbacks.keep(&mut sizes, n, Some(false), &style.font_size_px);
        if n == important
            && let Some(animated) = animated
        {
            let reverted = Rollbacks::kept(&sizes, rollbacks.user_agent(&sorted)).copied();
            animated.font_size(&mut style, parent, resolution, reverted);
            rollbacks.keep(&mut sizes, n, Some(true), &style.font_size_px);
        }
        // The `font` shorthand sets the size too (D53).
        let shorthand = match &*a.decl.name {
            "font-size" | "all" => false,
            "font" => true,
            _ => continue,
        };
        // A registered property of the font's units read is a cycle: the
        // declaration is invalid at computed-value time, `unset`, and so
        // is the property (D73).
        let units: &[&str] = if resolution.root {
            &FONT_UNITS_ON_ROOT
        } else {
            &FONT_UNITS
        };
        let read = reads_cycle(
            &a.decl.value,
            &declared,
            style.custom.as_deref(),
            resolution,
            units,
        );
        if !read.is_empty() {
            cyclic.extend(read);
            invalid.push(n);
            style.font_size_px = parent.font_size_px;
            continue;
        }
        let specified = specified(a, style.custom.as_ref(), resolution);
        if let Some(keyword) = specified.revert() {
            // The size where the keyword rolls back to; `unset`, which is
            // `inherit`, where nothing was.
            let mark = rollbacks.mark(&sorted, n, keyword);
            style.font_size_px = Rollbacks::kept(&sizes, mark)
                .copied()
                .unwrap_or(parent.font_size_px);
            continue;
        }
        if let Some(px) = declared_font_size(specified, shorthand, parent, resolution) {
            style.font_size_px = px;
        }
    }
    if important == sorted.len()
        && let Some(animated) = animated
    {
        rollbacks.keep(&mut sizes, important, Some(false), &style.font_size_px);
        let reverted = Rollbacks::kept(&sizes, rollbacks.user_agent(&sorted)).copied();
        animated.font_size(&mut style, parent, resolution, reverted);
    }
    for transition in transitioned {
        if let (Property::Longhand(Longhand::FontSize), Value::Style(start), Value::Style(end)) =
            (&transition.property, &transition.start, &transition.end)
        {
            animate::blend(
                Longhand::FontSize,
                start,
                end,
                transition.progress,
                &mut style,
            );
        }
    }
    // The rest of the font and `line-height` next, as the pass below
    // applies them: the font-relative units of every other property
    // measure the element's font and line (Values 4 section 6.1.1).
    // The pass below applies them again, in their order with any
    // animation. On the root, `rem` in `line-height` is the root's own
    // size, the root units the initial font's.
    let line_resolution_on_root;
    let line_resolution = if resolution.root && target == Target::Element {
        line_resolution_on_root = Resolution {
            rem: style.font_size_px,
            ..*resolution
        };
        &line_resolution_on_root
    } else {
        resolution
    };
    for pass in [FontPass::Font, FontPass::Line] {
        let font = style.font.clone();
        let own = LengthBases {
            em: style.font_size_px,
            font: font.as_deref().unwrap_or(crate::computed::initial_font()),
            line: (parent.font(), parent.font_size_px),
            resolution: match pass {
                FontPass::Font => resolution,
                FontPass::Line => line_resolution,
            },
            color: parent.color,
            dark: style.dark,
        };
        let of_parent = LengthBases::of_parent(parent, resolution);
        let mut kept: Vec<(Mark, ComputedStyle)> = Vec::new();
        for (n, a) in sorted.iter().enumerate() {
            rollbacks.keep(&mut kept, n, None, &style);
            let name = canonical_name(&a.decl.name);
            if FontPass::of(name) != Some(pass) || !target.applies(name) {
                continue;
            }
            let bases = font_bases(name, &own, &of_parent, &own);
            // `line-height` reading a registered property of the line's
            // units is a cycle (D73).
            let units: &[&str] = if resolution.root {
                &["lh", "rlh"]
            } else {
                &["lh"]
            };
            let read = match name {
                "line-height" => reads_cycle(
                    &a.decl.value,
                    &declared,
                    style.custom.as_deref(),
                    resolution,
                    units,
                ),
                _ => Vec::new(),
            };
            if !read.is_empty() || invalid.contains(&n) {
                cyclic.extend(read);
                if !invalid.contains(&n) {
                    invalid.push(n);
                }
                apply_css_wide(&mut style, name, CssWide::Unset, parent, &mut [false; 4]);
                continue;
            }
            let specified = specified(a, style.custom.as_ref(), resolution);
            if let Some(keyword) = specified.revert() {
                let mark = rollbacks.mark(&sorted, n, keyword);
                let reverted = Rollbacks::kept(&kept, mark);
                apply_revert(&mut style, name, reverted, parent, &mut [false; 4]);
                continue;
            }
            apply_specified(&mut style, name, specified, bases, parent, &mut [false; 4]);
        }
        // A transition of `line-height` moves what `lh` is.
        for transition in transitioned.iter().filter(|_| pass == FontPass::Line) {
            if let (
                Property::Longhand(Longhand::LineHeight),
                Value::Style(start),
                Value::Style(end),
            ) = (&transition.property, &transition.start, &transition.end)
                && target.applies("line-height")
            {
                animate::blend(
                    Longhand::LineHeight,
                    start,
                    end,
                    transition.progress,
                    &mut style,
                );
            }
        }
    }
    // On the root element `rem` and the root units are the initial
    // font's in the font properties alone; past them, the root's own
    // (Values 4 section 6.1.1).
    let root_resolution;
    let root_font = style.font.clone();
    let font_resolution = resolution;
    let resolution = if resolution.root && target == Target::Element {
        root_resolution = Resolution {
            rem: style.font_size_px,
            root_font: Some(
                root_font
                    .as_deref()
                    .unwrap_or(crate::computed::initial_font()),
            ),
            root_size: style.font_size_px,
            ..*resolution
        };
        &root_resolution
    } else {
        resolution
    };

    // A registered custom property in a cycle with the font or the line
    // is invalid at computed-value time: `unset` (D73).
    if !cyclic.is_empty()
        && let Some(custom) = style.custom.clone()
    {
        let registry = &resolution.substitutions.registry;
        let values: Vec<(&str, Arc<[Token]>)> = cyclic
            .iter()
            .filter_map(|&name| {
                let registration = registry.get(name)?;
                let inherited = parent.custom.as_ref().and_then(|m| m.get(name)).cloned();
                let value = if registration.inherits {
                    inherited.or_else(|| registration.initial.clone())
                } else {
                    registration.initial.clone()
                };
                value.map(|value| (name, value))
            })
            .collect();
        style.custom = Some(Arc::new(custom.with_values(values)));
    }

    // The registered custom properties the element declares, computed by
    // their syntaxes against its font, its line and the viewport (D73),
    // before any other value substitutes them; those it inherits are
    // its parent's computed values. Those of a colour of `currentcolor`
    // are computed once the element's colour is.
    let registry = &resolution.substitutions.registry;
    let mut current_colored: Vec<&str> = Vec::new();
    if !registry.is_empty()
        && let Some(custom) = style.custom.clone()
    {
        let font = style.font.clone();
        let font = font.as_deref().unwrap_or(crate::computed::initial_font());
        let bases = LengthBases {
            em: style.font_size_px,
            font,
            line: (font, style.font_size_px),
            resolution,
            color: style.color,
            dark: style.dark,
        };
        let mut values: Vec<(&str, Arc<[Token]>)> = Vec::new();
        // An initial value of the viewport's units, against this one.
        for registration in registry.iter().filter(|r| r.viewport) {
            let name = registration.name.as_str();
            if custom.get(name) != registration.initial.as_ref() {
                continue;
            }
            if let Some(initial) = &registration.initial_text
                && let Some(computed) = registered::compute(&registration.syntax, initial, &bases)
            {
                values.push((name, Arc::from(computed)));
            }
        }
        for declaration in &declared {
            let name = declaration.name;
            let Some(registration) = registry.get(name).filter(|r| !r.syntax.is_universal()) else {
                continue;
            };
            if values.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let Some(value) = custom.get(name) else {
                continue;
            };
            // A colour of `currentcolor` waits for the element's colour.
            if registered::reads_current_color(&registration.syntax, value) {
                current_colored.push(name);
                continue;
            }
            let computed = registered::compute(&registration.syntax, value, &bases)
                .map(Arc::from)
                .or_else(|| registration.initial.clone());
            if let Some(computed) = computed
                && *computed != **value
            {
                values.push((name, computed));
            }
        }
        if !values.is_empty() {
            let mut changed: Vec<&str> = values.iter().map(|(name, _)| *name).collect();
            let mut map = Arc::new(custom.with_values(values));
            // A property not registered with a syntax that the element declares
            // and that reads one of them substitutes its computed value, through
            // a chain of such properties a few deep.
            for _ in 0..MAX_RESUBSTITUTION {
                let mut more: Vec<(&str, Arc<[Token]>)> = Vec::new();
                for (at, declaration) in declared.iter().enumerate() {
                    let name = declaration.name;
                    let last = !declared[at + 1..].iter().any(|d| d.name == name);
                    if !last
                        || registry.get(name).is_some_and(|r| !r.syntax.is_universal())
                        || !custom::var_names(declaration.value).any(|n| changed.contains(&n))
                    {
                        continue;
                    }
                    if let Substituted::Value(value) = resolution.substitutions.substitute(
                        declaration.value,
                        None,
                        Some(&map),
                        resolution.attributes,
                        true,
                    ) {
                        more.push((name, value));
                    }
                }
                if more.is_empty() {
                    break;
                }
                changed = more.iter().map(|(name, _)| *name).collect();
                map = Arc::new(map.with_values(more));
            }
            style.custom = Some(map);
        }
        // Then the animations' values of them, interpolated by their
        // types, below the important declarations; then the transitions'.
        let animations = animated.map(|a| a.typed_custom(&typed)).unwrap_or_default();
        let important_names: Vec<&str> = sorted[important..]
            .iter()
            .map(|a| &*a.decl.name)
            .filter(|name| typed(name))
            .collect();
        for mix in animations {
            if important_names.contains(&mix.name) {
                continue;
            }
            let Some(registration) = registry.get(mix.name) else {
                continue;
            };
            let current = style.custom.clone();
            let underlying = current.as_ref().and_then(|m| m.get(mix.name)).cloned();
            let inherited = parent
                .custom
                .as_ref()
                .and_then(|m| m.get(mix.name))
                .cloned();
            // A keyframe's value computed, `None` for the underlying one.
            let endpoint = |value: Option<&[Token]>| -> Option<Arc<[Token]>> {
                let Some(tokens) = value else {
                    return underlying.clone();
                };
                let unset = || {
                    if registration.inherits {
                        inherited.clone().or_else(|| registration.initial.clone())
                    } else {
                        registration.initial.clone()
                    }
                };
                match css_wide_keyword(tokens) {
                    Some(CssWide::Initial) => return registration.initial.clone(),
                    Some(CssWide::Inherit) => {
                        return inherited.clone().or_else(|| registration.initial.clone());
                    }
                    Some(CssWide::Unset) => return unset(),
                    None if revert_keyword(tokens).is_some() => return unset(),
                    None => {}
                }
                let substituted = match resolution.substitutions.substitute(
                    tokens,
                    None,
                    current.as_ref(),
                    resolution.attributes,
                    true,
                ) {
                    Substituted::Value(value) => value,
                    _ => return unset(),
                };
                registered::compute(&registration.syntax, &substituted, &bases)
                    .map(Arc::from)
                    .or_else(unset)
            };
            // Whether a keyframe's value reads an attribute, by its own
            // `attr()` or a `var()` of a value that did; the underlying
            // one as it does (Values 5 section 7.7.3).
            let reads = |value: Option<&[Token]>| match value {
                None => current
                    .as_ref()
                    .is_some_and(|m| m.is_attr_tainted(mix.name)),
                Some(tokens) => {
                    custom::reads_attributes(tokens)
                        || custom::var_names(tokens)
                            .any(|name| current.as_ref().is_some_and(|m| m.is_attr_tainted(name)))
                }
            };
            let (from, to) = (endpoint(mix.from), endpoint(mix.to));
            let stepped = || {
                if mix.p < 0.5 {
                    (from.clone(), reads(mix.from))
                } else {
                    (to.clone(), reads(mix.to))
                }
            };
            let (value, attr) = match (&from, &to) {
                (Some(a), Some(b)) => match crate::property::interpolate(a, b, mix.p) {
                    Some(value) => (Some(Arc::from(value)), reads(mix.from) || reads(mix.to)),
                    None => stepped(),
                },
                _ => stepped(),
            };
            if let (Some(map), Some(value)) = (current.as_ref(), value) {
                let mut map = map.with_values(vec![(mix.name, value)]);
                map.set_attr_tainted(mix.name, attr);
                style.custom = Some(Arc::new(map));
            }
        }
        for transition in transitioned {
            if let (
                Property::Custom(name),
                Value::Custom(Some(start), _),
                Value::Custom(Some(end), _),
            ) = (&transition.property, &transition.start, &transition.end)
                && typed(name)
                && let Some(value) = crate::property::interpolate(start, end, transition.progress)
                && let Some(map) = style.custom.clone()
            {
                style.custom = Some(Arc::new(map.with_values(vec![(name, Arc::from(value))])));
            }
        }
    }

    // `direction` next (D74): what the flow-relative properties below
    // name the physical ones by, whatever their order with it. The user
    // agent's sheet sets it from `dir` (HTML section 15.3.5); the pass
    // below applies it again where it stands, to the same value.
    let mut directions = Vec::new();
    for (n, a) in sorted.iter().enumerate() {
        rollbacks.keep(&mut directions, n, None, &style.direction);
        if a.decl.name != "direction" || !target.applies("direction") {
            continue;
        }
        let specified = specified(a, style.custom.as_ref(), resolution);
        if let Some(keyword) = specified.revert() {
            let mark = rollbacks.mark(&sorted, n, keyword);
            style.direction = Rollbacks::kept(&directions, mark)
                .copied()
                .unwrap_or(parent.direction);
            continue;
        }
        let direction = match specified {
            Specified::Written(value) => parse_direction_value(value, parent),
            Specified::Substituted(value) => {
                Some(parse_direction_value(&value, parent).unwrap_or(parent.direction))
            }
            Specified::Invalid => Some(parent.direction),
            Specified::Ignored => None,
        };
        if let Some(direction) = direction {
            style.direction = direction;
        }
    }

    // `color-scheme` next (gap batch E, D49): the scheme the element's
    // system colours and `light-dark()` resolve by.
    let mut schemes = Vec::new();
    for (n, a) in sorted.iter().enumerate() {
        rollbacks.keep(&mut schemes, n, None, &style.color_scheme);
        if a.decl.name != "color-scheme" || !target.applies("color-scheme") {
            continue;
        }
        let specified = specified(a, style.custom.as_ref(), resolution);
        if let Some(keyword) = specified.revert() {
            let mark = rollbacks.mark(&sorted, n, keyword);
            style.color_scheme = Rollbacks::kept(&schemes, mark)
                .cloned()
                .unwrap_or_else(|| parent.color_scheme.clone());
            continue;
        }
        let scheme = match specified {
            Specified::Written(value) => color_scheme_of(value, parent),
            Specified::Substituted(value) => {
                Some(color_scheme_of(&value, parent).unwrap_or(parent.color_scheme.clone()))
            }
            Specified::Invalid => Some(parent.color_scheme.clone()),
            Specified::Ignored => None,
        };
        if let Some(scheme) = scheme {
            style.color_scheme = scheme;
        }
    }
    style.dark = uses_dark(
        style.color_scheme.as_deref().map(String::as_str),
        resolution,
    );
    // The root's `color` is the initial `CanvasText`, by its scheme.
    if resolution.root {
        style.color = crate::color::system("canvastext", style.dark).unwrap_or(Color::BLACK);
    }

    // `color` next (gap batch E): `currentcolor` in every other
    // property is the element's computed `color`, its own `currentcolor`
    // the parent's.
    // Each declaration's colour is kept, by its place, for the pass
    // below, which does not substitute and parse it again.
    let mut current = style.color;
    let mut declared: Vec<(usize, Color)> = Vec::new();
    let mut colors = Vec::new();
    // A relative length in its components is of the element's font, final
    // now (D78).
    let font = style.font.clone();
    let font = font.as_deref().unwrap_or(crate::computed::initial_font());
    let color_bases = LengthBases {
        em: style.font_size_px,
        font,
        line: (font, style.font_size_px),
        resolution,
        color: parent.color,
        dark: style.dark,
    };
    for (n, a) in sorted.iter().enumerate() {
        rollbacks.keep(&mut colors, n, None, &current);
        if !matches!(&*a.decl.name, "color" | "all") || !target.applies("color") {
            continue;
        }
        let specified = specified(a, style.custom.as_ref(), resolution);
        if let Some(keyword) = specified.revert() {
            let mark = rollbacks.mark(&sorted, n, keyword);
            current = Rollbacks::kept(&colors, mark)
                .copied()
                .unwrap_or(parent.color);
            declared.push((n, current));
            continue;
        }
        let color = match specified {
            Specified::Written(value) => color_of_color(value, parent, &color_bases),
            Specified::Substituted(value) => {
                Some(color_of_color(&value, parent, &color_bases).unwrap_or(parent.color))
            }
            Specified::Invalid => Some(parent.color),
            Specified::Ignored => None,
        };
        if let Some(c) = color {
            current = c;
            declared.push((n, c));
        }
    }
    // The registered colours of `currentcolor`, against the element's
    // colour now known (D73).
    if !current_colored.is_empty()
        && let Some(custom) = style.custom.clone()
    {
        let font = style.font.clone();
        let font = font.as_deref().unwrap_or(crate::computed::initial_font());
        let bases = LengthBases {
            em: style.font_size_px,
            font,
            line: (font, style.font_size_px),
            resolution,
            color: current,
            dark: style.dark,
        };
        let registry = &resolution.substitutions.registry;
        let values: Vec<(&str, Arc<[Token]>)> = current_colored
            .iter()
            .filter_map(|&name| {
                let registration = registry.get(name)?;
                let value = custom.get(name)?;
                let computed = registered::compute(&registration.syntax, value, &bases)?;
                Some((name, Arc::from(computed)))
            })
            .collect();
        style.custom = Some(Arc::new(custom.with_values(values)));
    }
    // Kept only where an animation or a transition may move `color`.
    let moves = animated.is_some()
        || transitioned.iter().any(|t| {
            matches!(
                t.property,
                Property::Longhand(Longhand::Color) | Property::Custom(_)
            )
        });
    let start = moves.then(|| style.clone());
    let mut border_color_set = [false; 4];
    let apply_all = |style: &mut ComputedStyle, border_color_set: &mut [bool; 4], color| {
        let font = style.font.clone();
        let font = font.as_deref().unwrap_or(crate::computed::initial_font());
        let bases = LengthBases {
            em: style.font_size_px,
            font,
            line: (font, style.font_size_px),
            resolution,
            color,
            dark: style.dark,
        };
        // The font properties' and `line-height`'s on the root are the
        // initial font's.
        let of_parent = LengthBases {
            color,
            ..LengthBases::of_parent(parent, font_resolution)
        };
        let line = LengthBases {
            line: (parent.font(), parent.font_size_px),
            resolution: line_resolution,
            ..bases
        };
        // The style at each place a revert keyword rolls back to.
        let mut kept: Vec<(Mark, ComputedStyle)> = Vec::new();
        for (n, a) in sorted.iter().enumerate() {
            rollbacks.keep(&mut kept, n, Some(false), style);
            if n == important
                && let Some(animated) = animated
            {
                let reverted = Rollbacks::kept(&kept, rollbacks.user_agent(&sorted));
                animated.apply(style, parent, target, &bases, border_color_set, reverted);
                rollbacks.keep(&mut kept, n, Some(true), style);
            }
            let name = canonical_name(&a.decl.name);
            if custom::is_custom_property_name(name) || !target.applies(name) {
                continue;
            }
            if name == "color" || name == "all" {
                if let Some(&(_, c)) = declared.iter().find(|(at, _)| *at == n) {
                    style.color = c;
                }
                if name == "color" {
                    continue;
                }
            }
            // A declaration in a cycle with the font or the line (D73).
            if invalid.contains(&n) {
                apply_css_wide(style, name, CssWide::Unset, parent, border_color_set);
                continue;
            }
            let specified = specified(a, style.custom.as_ref(), resolution);
            if let Some(keyword) = specified.revert() {
                let mark = rollbacks.mark(&sorted, n, keyword);
                let reverted = Rollbacks::kept(&kept, mark);
                apply_revert(style, name, reverted, parent, border_color_set);
                continue;
            }
            apply_specified(
                style,
                name,
                specified,
                font_bases(name, &bases, &of_parent, &line),
                parent,
                border_color_set,
            );
        }
        if important == sorted.len()
            && let Some(animated) = animated
        {
            rollbacks.keep(&mut kept, important, Some(false), style);
            let reverted = Rollbacks::kept(&kept, rollbacks.user_agent(&sorted));
            animated.apply(style, parent, target, &bases, border_color_set, reverted);
        }
        for transition in transitioned {
            if let (Property::Longhand(longhand), Value::Style(start), Value::Style(end)) =
                (&transition.property, &transition.start, &transition.end)
                && *longhand != Longhand::FontSize
                && target.applies(longhand.name())
            {
                animate::blend(*longhand, start, end, transition.progress, style);
                let side = match longhand {
                    Longhand::BorderTopColor => Some(TOP),
                    Longhand::BorderRightColor => Some(RIGHT),
                    Longhand::BorderBottomColor => Some(BOTTOM),
                    Longhand::BorderLeftColor => Some(LEFT),
                    _ => None,
                };
                if let Some(side) = side {
                    border_color_set[side] = true;
                }
            }
        }
    };
    apply_all(&mut style, &mut border_color_set, current);
    // An animation or a transition of `color` changes what
    // `currentcolor` is: the rest are computed again with it.
    if let Some(start) = start
        && style.color != current
    {
        let color = style.color;
        style = start;
        border_color_set = [false; 4];
        apply_all(&mut style, &mut border_color_set, color);
    }
    // `normal` computes to `none` on `::before` and `::after`. On an
    // element `content` is an image alone, which replaces what it shows
    // (M23.3), or `normal`.
    if target.takes_content() && style.content == Content::Normal {
        style.content = Content::None;
    }
    if target == Target::Element
        && !matches!(&style.content, Content::Items(items) if matches!(**items, [ContentItem::Image(_)]))
    {
        style.content = Content::Normal;
    }
    for (side, set) in border_color_set.into_iter().enumerate() {
        if !set {
            // `border-color`'s initial value is `currentcolor` (css-color-4
            // section 4.4).
            style.border_color[side] = Color::CURRENT;
        }
    }
    // Every `currentcolor`, the inherited ones among them, is the
    // element's own `color`, final now (D66, G226).
    style.resolve_current_color();
    // CSS2 §9.7, the relationships between `display`, `position` and
    // `float` (M11): an absolutely positioned box does not float, and
    // a float or an absolutely positioned box takes the block-level
    // `display` of §9.7's table (M12: an inline table becomes a table,
    // a table-internal box a block), so the box tree sees a block
    // container or a table. (The root element's `float` is `none` too;
    // the box tree builder keeps the top-level boxes in flow.)
    if style.position.is_out_of_flow() {
        style.float = Float::None;
        style.flow_relative.float = None;
    }
    style.inline_out_of_flow = style.position.is_out_of_flow()
        && matches!(
            style.display,
            Display::Inline
                | Display::InlineBlock
                | Display::InlineTable
                | Display::InlineFlex
                | Display::InlineGrid
                | Display::InlineListItem
                | Display::Ruby
                | Display::Math
        );
    if style.float != Float::None || style.position.is_out_of_flow() {
        style.display = style.display.blockified();
    }
    // The children of a flex or a grid container are its items,
    // blockified (css-display-3 §2.7, css-flexbox-1 §4, M17; css-grid-2
    // §6.1, M18): an element and its `::before` and `::after`, whose
    // parent here is the element. A `::first-letter` or `::first-line`
    // is not one, nor is an element inheriting from a `::first-line`,
    // whose `display` is never a flex or a grid container's.
    // A parent of `display: contents` makes no box: the box its children
    // are in is the nearest ancestor's that makes one (css-display-3
    // section 2.5, D77), which the parent's style records.
    let in_items = parent.display.has_items()
        || (parent.display == Display::Contents && parent.contents_in_items);
    if matches!(target, Target::Element | Target::Before | Target::After) && in_items {
        style.display = style.display.blockified();
    }
    style.contents_in_items = style.display == Display::Contents && in_items;
    // `math` on a pseudo-element computes to the flow value (MathML Core
    // section 2.1).
    if matches!(target, Target::Before | Target::After | Target::Marker) {
        style.display = style.display.without_math();
    }
    // A marker (Lists 3 section 3.1, as the reference browser computes
    // it): an outside one an inline block, an inside one inline, neither
    // indented and each aligned at its start, whatever its list item's
    // text says.
    if target == Target::Marker {
        style.display = match parent.lists().list_style_position {
            crate::computed::ListStylePosition::Outside => Display::InlineBlock,
            crate::computed::ListStylePosition::Inside => Display::Inline,
        };
        style.text_align = crate::computed::TextAlign::Start;
        style.legacy_align = crate::computed::LegacyAlign::None;
        if style.text().text_indent != crate::computed::TextIndent::default() {
            style.text_mut().text_indent = crate::computed::TextIndent::default();
        }
    }
    // A spacing of a percentage is of the element's own font size, which
    // an inherited one is resolved against again.
    if let Some(text) = style.text.as_deref() {
        let em = style.font_size_px;
        let letter = text.letter_spacing.as_ref().map(|s| s.resolve(em));
        let word = text.word_spacing.as_ref().map(|s| s.resolve(em));
        if let Some(px) = letter {
            style.letter_spacing = px;
        }
        if let Some(px) = word {
            style.word_spacing = px;
        }
    }
    if style
        .text
        .as_deref()
        .is_some_and(|t| *t == crate::computed::TextProps::default())
    {
        style.text = None;
    }
    // Grid and effects properties set back to their initial values leave
    // no copy of them behind, so that two styles alike compare equal. The
    // initial values are made only for a style that holds a copy: most
    // hold none, and `Motion`'s alone allocates thirteen lists.
    if style
        .grid
        .as_deref()
        .is_some_and(|g| *g == GridStyle::default())
    {
        style.grid = None;
    }
    if style
        .effects
        .as_deref()
        .is_some_and(|e| *e == Effects::default())
    {
        style.effects = None;
    }
    if style
        .fragmentation
        .as_deref()
        .is_some_and(|f| *f == Fragmentation::default())
    {
        style.fragmentation = None;
    }
    if style
        .lists
        .as_deref()
        .is_some_and(|l| *l == Lists::default())
    {
        style.lists = None;
    }
    if style
        .decoration
        .as_deref()
        .is_some_and(|d| *d == Decoration::default())
    {
        style.decoration = None;
    }
    // The tree `animation-name` was given in, a shadow tree's, where its
    // names are looked up first (CSS Scoping 1 section 3.5, D69).
    if style
        .motion
        .as_deref()
        .is_some_and(|m| m.animation_name.iter().any(Option::is_some))
        && let Some(tree) = sorted
            .iter()
            .rev()
            .find(|a| matches!(canonical_name(&a.decl.name), "animation-name" | "animation"))
            .and_then(|a| a.tree())
    {
        style.motion_mut().animation_names_tree = Some(tree);
    }
    if style
        .motion
        .as_deref()
        .is_some_and(|m| *m == Motion::default())
    {
        style.motion = None;
    }
    style.fix_overflow();
    if style.svg.as_deref().is_some_and(|s| *s == Svg::default()) {
        style.svg = None;
    }
    if style
        .radii
        .as_deref()
        .is_some_and(|r| *r == Radii::default())
    {
        style.radii = None;
    }
    style
}

/// The properties applied ahead of the rest, so that the rest's
/// font-relative units measure the element's font and line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontPass {
    /// The font's family, weight, slant, width and features.
    Font,
    /// `line-height`, which `lh` is.
    Line,
}

impl FontPass {
    /// The pass the property `name` is applied in ahead, if any.
    fn of(name: &str) -> Option<FontPass> {
        match name {
            "font-family"
            | "font"
            | "font-weight"
            | "font-style"
            | "font-stretch"
            | "font-feature-settings" => Some(FontPass::Font),
            "line-height" => Some(FontPass::Line),
            _ => None,
        }
    }
}

/// The bases the property `name` reads its lengths against: the font
/// longhands the parent's (Values 4 section 6.1.1), `line-height` the
/// element's but for `lh`, the parent's line, and the rest `own`.
fn font_bases<'b, 'a>(
    name: &str,
    own: &'b LengthBases<'a>,
    of_parent: &'b LengthBases<'a>,
    line: &'b LengthBases<'a>,
) -> &'b LengthBases<'a> {
    match name {
        "font-weight" | "font-style" | "font-stretch" | "font-feature-settings" => of_parent,
        "line-height" => line,
        _ => own,
    }
}

/// Applies the declaration of `name` as `specified` gives it. A value
/// substituted that its property does not take is invalid at
/// computed-value time, the property `unset`: its inherited or initial
/// value, not a value it beat in the cascade (Values 5 Appendix A). A
/// shorthand leaves each of its longhands so.
fn apply_specified(
    style: &mut ComputedStyle,
    name: &str,
    specified: Specified<'_>,
    bases: &LengthBases<'_>,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) {
    match specified {
        Specified::Written(value) => {
            apply_declaration(style, name, value, bases, parent, border_color_set);
        }
        Specified::Substituted(value) => {
            if !apply_declaration(style, name, &value, bases, parent, border_color_set) {
                apply_css_wide(style, name, CssWide::Unset, parent, border_color_set);
            }
        }
        Specified::Invalid => {
            apply_css_wide(style, name, CssWide::Unset, parent, border_color_set);
        }
        Specified::Ignored => {}
    }
}

/// The font size a `font-size` declaration whose value is `specified`
/// gives an element whose parent's style is `parent`: its own `em` and
/// percentage of the parent's; `unset`, which is `inherit`, when it is
/// invalid at computed-value time; `None` when it is ignored. Of a
/// `font` declaration when `shorthand`.
fn declared_font_size(
    specified: Specified<'_>,
    shorthand: bool,
    parent: &ComputedStyle,
    resolution: &Resolution<'_>,
) -> Option<f32> {
    let parent_bases = LengthBases::of_parent(parent, resolution);
    let font_size = |value: &[Token]| match css_wide_keyword(value) {
        Some(CssWide::Initial) => Some(ComputedStyle::default().font_size_px),
        // `font-size` is inherited, so `unset` means `inherit`.
        Some(CssWide::Inherit | CssWide::Unset) => Some(parent.font_size_px),
        None if shorthand => {
            fonts::parse_font(value, parent.font().weight, &parent_bases).map(|f| f.size)
        }
        None => parse_font_size(value, &parent_bases),
    };
    match specified {
        Specified::Written(value) => font_size(value),
        // Invalid at computed-value time: `unset`, which is `inherit`.
        Specified::Substituted(value) => Some(font_size(&value).unwrap_or(parent.font_size_px)),
        Specified::Invalid => Some(parent.font_size_px),
        Specified::Ignored => None,
    }
}

/// A declaration's value as the cascade applies it (M21.1).
enum Specified<'v> {
    /// The value as written, with no substitution function in it.
    Written(&'v [Token]),
    /// The value with its substitution functions replaced: one its
    /// property does not take is invalid at computed-value time.
    Substituted(Arc<[Token]>),
    /// The substitution gave the guaranteed-invalid value: invalid at
    /// computed-value time.
    Invalid,
    /// A substitution function in it is invalid by its grammar: the
    /// declaration is invalid at parse time, ignored.
    Ignored,
}

/// The units relative to an element's font, which its `font-size` may
/// not read a registered custom property of (D73).
const FONT_UNITS: [&str; 7] = ["em", "ex", "ch", "cap", "ic", "lh", "rlh"];

/// [`FONT_UNITS`] on the root element, whose own font the root units
/// are.
const FONT_UNITS_ON_ROOT: [&str; 12] = [
    "em", "ex", "ch", "cap", "ic", "lh", "rlh", "rem", "rex", "rch", "rcap", "ric",
];

/// The custom properties registered with a syntax that `value`, an
/// element's declaration of `font-size` or `line-height`, reads through
/// `var()` and that the element declares, among `declared`, with a unit
/// of `units` in its value, `custom`'s: such a property computes against
/// the very font or line the declaration gives, a cycle that makes both
/// invalid at computed-value time (Properties and Values API section
/// 2.4, D73). A property the element inherits is computed already; one
/// not registered is followed to what it reads. Empty for no cycle.
fn reads_cycle<'d>(
    value: &'d [Token],
    declared: &[custom::Declaration<'d>],
    custom: Option<&CustomProperties>,
    resolution: &Resolution<'_>,
    units: &[&str],
) -> Vec<&'d str> {
    dom::hook!("style::reads_cycle");
    let registry = &resolution.substitutions.registry;
    let mut cyclic = Vec::new();
    if registry.is_empty() || !custom::has_substitution(value) {
        return cyclic;
    }
    let mut names: Vec<&str> = custom::var_names(value).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(name) = names.pop() {
        if !seen.insert(name) || seen.len() > MAX_CYCLE_WALK {
            continue;
        }
        let Some(declaration) = declared.iter().rev().find(|d| d.name == name) else {
            continue;
        };
        if registry.get(name).is_some_and(|r| !r.syntax.is_universal()) {
            let relative = custom.and_then(|m| m.get(name)).is_some_and(|tokens| {
                tokens.iter().any(|t| {
                    matches!(t, Token::Dimension { unit, .. }
                        if units.iter().any(|u| unit.eq_ignore_ascii_case(u)))
                })
            });
            if relative {
                cyclic.push(declaration.name);
            }
            continue;
        }
        names.extend(custom::var_names(declaration.value));
    }
    cyclic
}

/// How many custom properties [`reads_cycle`] follows from one value.
const MAX_CYCLE_WALK: usize = 1024;

/// How deep a chain of properties not registered is followed to give
/// each the computed value of a registered one it reads (D73).
const MAX_RESUBSTITUTION: usize = 8;

impl Specified<'_> {
    /// The revert keyword the value is, written or substituted (D73).
    fn revert(&self) -> Option<Revert> {
        match self {
            Specified::Written(value) => revert_keyword(value),
            Specified::Substituted(value) => revert_keyword(value),
            Specified::Invalid | Specified::Ignored => None,
        }
    }
}

/// Applies a revert keyword of the property `name` (D73): its value as
/// the style `reverted` has it, which the cascade rolls back to; `unset`
/// where there is none, the cascade rolling back past every declaration.
fn apply_revert(
    style: &mut ComputedStyle,
    name: &str,
    reverted: Option<&ComputedStyle>,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) {
    dom::hook!("style::apply_revert");
    match reverted {
        Some(reverted) => {
            apply_css_wide(style, name, CssWide::Inherit, reverted, border_color_set);
        }
        None => {
            apply_css_wide(style, name, CssWide::Unset, parent, border_color_set);
        }
    }
}

/// The value of `a` as the cascade applies it, its substitution functions
/// replaced against the custom properties `custom`.
fn specified<'v>(
    a: &'v Applied<'_>,
    custom: Option<&Arc<CustomProperties>>,
    resolution: &Resolution<'_>,
) -> Specified<'v> {
    // A `::part()` rule's `attr()` reads its originating host's.
    let of_host;
    let resolution = match (a.originating(), resolution.containers) {
        (Some(host), Some(containers)) => {
            of_host = Resolution {
                attributes: containers.document.attrs(host).unwrap_or_default(),
                ..*resolution
            };
            &of_host
        }
        _ => resolution,
    };
    substituted(
        &a.decl.value,
        a.identity(),
        canonical_name(&a.decl.name),
        custom,
        resolution,
    )
}

/// `value`, a declaration's of the property `name` whose place in memory
/// is `identity`, as the cascade applies it: its substitution functions
/// replaced against the custom properties `custom`, a `var()` of an
/// animation-tainted one the guaranteed-invalid value in a property that
/// is not animatable (Variables 1 §3, M22).
fn substituted<'v>(
    value: &'v [Token],
    identity: Option<usize>,
    name: &str,
    custom: Option<&Arc<CustomProperties>>,
    resolution: &Resolution<'_>,
) -> Specified<'v> {
    if !custom::has_substitution(value) {
        return Specified::Written(value);
    }
    let animatable = custom.is_none_or(|m| !m.has_taint())
        || crate::animate::longhands_of(name)
            .iter()
            .any(|longhand| longhand.is_animatable());
    match resolution.substitutions.substitute(
        value,
        identity,
        custom,
        resolution.attributes,
        animatable,
    ) {
        // An attribute's value never becomes a URL (Values 5's attr()
        // taint, M23.3).
        Substituted::Value(_) if custom::url_from_attribute(value, custom) => Specified::Invalid,
        Substituted::Value(value) => Specified::Substituted(value),
        Substituted::Invalid => Specified::Invalid,
        Substituted::Ignored => Specified::Ignored,
    }
}

/// Whether the cascade accepts the declaration `name: value` (M17): a
/// property this engine knows, by its name or a `-webkit-` alias, and a
/// value that property's parser takes, a CSS-wide keyword included.
/// `name` and `value` are a declaration's as `css` parses one: the name
/// lowercased, a custom property's aside, the value's tokens trimmed and
/// without `!important`. The cascade's own parsers answer, so the answer
/// is what the cascade does with the declaration. A relative length
/// resolves against a 16px font in a viewport of no size,
/// `CascadeContext::default()`'s, which no answer depends on.
///
/// A custom property (M21.1) takes what [`crate::custom`] says its value
/// may be. A value with a substitution function in it is taken, where the
/// function is valid by its grammar, for any property this engine knows:
/// whether its property takes what it substitutes to is known only then
/// (Values 5 Appendix A).
pub fn supports(name: &str, value: &[Token]) -> bool {
    if custom::is_custom_property_name(name) {
        return custom::is_valid_custom_declaration(value);
    }
    let parent = ComputedStyle::default();
    // A revert keyword is a CSS-wide keyword too (D73), and a value with
    // a substitution function in it any value.
    if revert_keyword(value).is_some() || custom::has_substitution(value) {
        return custom::check(value)
            && apply_css_wide(
                &mut ComputedStyle::default(),
                canonical_name(name),
                CssWide::Initial,
                &parent,
                &mut [false; 4],
            );
    }
    with_default_bases(|bases| {
        apply_declaration(
            &mut ComputedStyle::default(),
            canonical_name(name),
            value,
            bases,
            &parent,
            &mut [false; 4],
        )
    })
}

/// Runs `f` with the bases a value is read against when no element's
/// are at hand: a 16px font in a viewport of no size,
/// `CascadeContext::default()`'s.
fn with_default_bases<R>(f: impl FnOnce(&LengthBases<'_>) -> R) -> R {
    let context = CascadeContext::default();
    let viewport_used = Cell::new(false);
    let substitutions = Substitutions::new(0);
    let resolution = Resolution {
        rem: 16.0,
        root: false,
        page_scheme: None,
        context: &context,
        viewport_used: &viewport_used,
        fonts_used: None,
        root_font: None,
        root_size: 16.0,
        scheme_read: None,
        attributes: &[],
        substitutions: &substitutions,
        containers: None,
        units_from: None,
        containers_used: &viewport_used,
    };
    f(&LengthBases {
        em: 16.0,
        font: crate::computed::initial_font(),
        line: (crate::computed::initial_font(), 16.0),
        resolution: &resolution,
        color: Color::BLACK,
        dark: false,
    })
}

/// Whether `tokens`, one component value, or a `<transform-list>`'s
/// values, are of the data type `data` as this engine reads it (M21.5:
/// what `attr()`'s `type()` asks of an attribute's value). A math
/// function is of a numeric type when it is typed so; a length is one of
/// the units this engine knows, or `0`; a color is one `color` takes. No
/// answer depends on what a relative length resolves to.
pub(crate) fn matches_type(data: DataType, tokens: &[Token]) -> bool {
    let math = |want| calc::is_math_function(tokens) && calc::parse(tokens, want).is_some();
    let measure = |measure| {
        matches!(single_token(tokens), Some(Token::Dimension { unit, .. })
            if calc::measure(unit) == Some(measure))
    };
    with_default_bases(|bases| match data {
        DataType::Length => length(tokens, bases, Range::All).is_some(),
        DataType::LengthPercentage => length_percentage(tokens, bases, Range::All).is_some(),
        DataType::Number => {
            matches!(single_token(tokens), Some(Token::Number { .. })) || math(Want::Number)
        }
        DataType::Integer => {
            matches!(
                single_token(tokens),
                Some(Token::Number {
                    is_integer: true,
                    ..
                })
            ) || math(Want::Number)
        }
        DataType::Percentage => {
            matches!(single_token(tokens), Some(Token::Percentage { .. })) || math(Want::Percentage)
        }
        DataType::Angle => measure(calc::Measure::Angle) || math(Want::Angle),
        DataType::Time => measure(calc::Measure::Time) || math(Want::Time),
        // Never negative (Values 4 section 7.4).
        DataType::Resolution => {
            (measure(calc::Measure::Resolution)
                && !matches!(single_token(tokens), Some(Token::Dimension { value, .. }) if *value < 0.0))
                || math(Want::Resolution)
        }
        // `currentcolor` among them.
        DataType::Color => color_with(tokens, Some(Color::CURRENT), false).is_some(),
        DataType::CustomIdent => {
            matches!(single_token(tokens), Some(Token::Ident(word)) if custom::is_custom_ident(word))
        }
        DataType::String => matches!(single_token(tokens), Some(Token::String { bad: false, .. })),
        DataType::TransformFunction => {
            effects::parse_transform(tokens, bases).is_some_and(|functions| functions.len() == 1)
        }
        DataType::TransformList => {
            single_ident(tokens).is_none()
                && effects::parse_transform(tokens, bases).is_some_and(|f| !f.is_empty())
        }
        // An image, `none` not among them (Images 3 section 2).
        DataType::Image => backgrounds::image(tokens, bases).is_some(),
        DataType::Url => is_url(tokens),
    })
}

/// Applies one already-cascade-ordered declaration to `style`, ignoring
/// (never erroring on) any property name or value this engine doesn't
/// understand yet — the M3 forward-compatibility pattern: a browser
/// encounters unsupported properties on every real page, constantly.
/// Says whether it applied it, which is what [`supports`] answers (M17).
fn apply_declaration(
    style: &mut ComputedStyle,
    name: &str,
    value: &[Token],
    bases: &LengthBases<'_>,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) -> bool {
    if let Some(keyword) = css_wide_keyword(value) {
        // `color`'s initial value is `CanvasText` by the scheme (D49).
        if name == "color" && matches!(keyword, CssWide::Initial) {
            style.color = crate::color::system("canvastext", bases.dark).unwrap_or(Color::BLACK);
            return true;
        }
        return apply_css_wide(style, name, keyword, parent, border_color_set);
    }
    if let Some(mapped) = logical::map(name, style.direction) {
        return apply_logical(style, mapped, value, bases, parent, border_color_set);
    }
    match name {
        name if timelines::is_timeline_property(name) => {
            timelines::apply(style, name, value, bases)
        }
        "color" => set(&mut style.color, color_of_color(value, parent, bases)),
        "direction" => set(&mut style.direction, parse_direction(value)),
        "color-scheme" => match color_scheme_of(value, parent) {
            Some(scheme) => {
                style.color_scheme = scheme;
                true
            }
            None => false,
        },
        "background-color" => {
            let applied = set(
                &mut style.background_color,
                color_value(value, bases.colors()),
            );
            if applied {
                keep_color_mix(style, mixes_current_color(value).then(|| Arc::from(value)));
            }
            applied
        }
        "background" => match backgrounds::parse_background(value, bases) {
            Some((color, layers)) => {
                style.background_color = color;
                style.background = (layers != Backgrounds::default()).then(|| Arc::new(layers));
                true
            }
            None => false,
        },
        "background-image" => set_background(
            style,
            backgrounds::parse_background_image(value, bases),
            |b, v| b.image = v,
        ),
        "background-repeat" => set_background(
            style,
            backgrounds::parse_background_repeat(value),
            |b, v| b.repeat = v,
        ),
        "background-position" => set_background(
            style,
            backgrounds::parse_background_position(value, bases),
            |b, v| b.set_positions(v),
        ),
        // Its longhands (Backgrounds 4, gap batch E; G40).
        "background-position-x" | "background-position-y" => {
            let x = name == "background-position-x";
            set_background(
                style,
                backgrounds::parse_background_position_axis(value, bases, x),
                |b, v| {
                    if x {
                        b.position_x = v;
                    } else {
                        b.position_y = v;
                    }
                },
            )
        }
        "background-size" => set_background(
            style,
            backgrounds::parse_background_size(value, bases),
            |b, v| b.size = v,
        ),
        "background-origin" => {
            set_background(style, backgrounds::parse_background_box(value), |b, v| {
                b.origin = v
            })
        }
        "background-clip" => {
            set_background(style, backgrounds::parse_background_box(value), |b, v| {
                b.clip = v
            })
        }
        "background-attachment" => set_background(
            style,
            backgrounds::parse_background_attachment(value),
            |b, v| b.attachment = v,
        ),
        // `border-radius` (Backgrounds 3 section 5, gap batch E, D50).
        "border-radius" => match parse_border_radius(value, bases) {
            Some(radii) => {
                style.radii = (radii != Radii::default()).then(|| Arc::new(radii));
                true
            }
            None => false,
        },
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius" => match parse_corner_radius(value, bases) {
            Some(corner) => {
                style.radii_mut().0[corner_of(name)] = corner;
                true
            }
            None => false,
        },
        "border-image" => match border_image::parse_border_image(value, bases) {
            Some(image) => {
                style.border_image = (image != BorderImage::default()).then(|| Arc::new(image));
                true
            }
            None => false,
        },
        "border-image-source" => set_border_image(
            style,
            border_image::parse_border_image_source(value, bases),
            |b, v| b.source = v,
        ),
        "border-image-slice" => set_border_image(
            style,
            border_image::parse_border_image_slice(value, bases),
            |b, (slice, fill)| {
                b.slice = slice;
                b.fill = fill;
            },
        ),
        "border-image-width" => set_border_image(
            style,
            border_image::parse_border_image_width(value, bases),
            |b, v| b.width = v,
        ),
        "border-image-outset" => set_border_image(
            style,
            border_image::parse_border_image_outset(value, bases),
            |b, v| b.outset = v,
        ),
        "border-image-repeat" => set_border_image(
            style,
            border_image::parse_border_image_repeat(value),
            |b, v| b.repeat = v,
        ),
        // The first pass of `compute_style` applies it.
        "font-size" => parse_font_size(value, bases).is_some(),
        "font-family" => set_font(style, parse_font_family(value).map(Into::into), |f, v| {
            f.family = v
        }),
        // The size is the first pass's.
        "font" => match fonts::parse_font(value, parent.font().weight, bases) {
            Some(font) => {
                style.font = (font.props != FontProps::default()).then(|| Arc::new(font.props));
                true
            }
            None => false,
        },
        "font-weight" => set_font(
            style,
            fonts::parse_font_weight(value, parent.font().weight, bases),
            |f, v| f.weight = v,
        ),
        "font-style" => set_font(style, fonts::parse_font_style(value, bases), |f, v| {
            f.style = v
        }),
        "font-stretch" => set_font(style, fonts::parse_font_stretch(value, bases), |f, v| {
            f.stretch = v
        }),
        "font-variant" => set_font(style, fonts::parse_font_variant(value), |f, v| {
            f.variant = Arc::new(v)
        }),
        "font-variant-ligatures"
        | "font-variant-caps"
        | "font-variant-alternates"
        | "font-variant-numeric"
        | "font-variant-east-asian"
        | "font-variant-position"
        | "font-variant-emoji" => set_font(style, fonts::parse_variant(name, value), |f, v| {
            if let Some(field) = fonts::variant_field(Arc::make_mut(&mut f.variant), name) {
                *field = v;
            }
        }),
        "font-feature-settings" => set_font(
            style,
            fonts::parse_font_feature_settings(value, bases),
            |f, v| f.features = v,
        ),
        "font-kerning" => set_font(
            style,
            single_ident(value).and_then(FontKerning::from_name),
            |f, v| f.kerning = v,
        ),
        "line-height" => set_font(style, fonts::parse_line_height(value, bases), |f, v| {
            f.line_height = v
        }),
        "display" => match parse_display(value) {
            Some((display, flow_root)) => {
                style.display = display;
                style.flow_root = flow_root;
                true
            }
            None => false,
        },
        "text-align" => {
            let legacy = match parse_text_align(value) {
                Some(TextAlignValue::JustifyAll) => {
                    style.text_align = TextAlign::Justify;
                    style.text_align_last = TextAlignLast::Align(TextAlign::Justify);
                    crate::computed::LegacyAlign::None
                }
                Some(TextAlignValue::MatchParent) => {
                    style.text_align = parent.text_align.matched(parent.direction);
                    style.text_align_last = matched_last(parent);
                    parent.legacy_align
                }
                Some(TextAlignValue::Align(align)) => {
                    style.text_align = align;
                    style.text_align_last = TextAlignLast::Auto;
                    crate::computed::LegacyAlign::None
                }
                Some(TextAlignValue::Legacy(align, legacy)) => {
                    style.text_align = align;
                    style.text_align_last = TextAlignLast::Auto;
                    legacy
                }
                None => return false,
            };
            style.legacy_align = legacy;
            true
        }
        "text-align-all" => match parse_text_align(value) {
            Some(TextAlignValue::Align(align) | TextAlignValue::Legacy(align, _)) => {
                set(&mut style.text_align, Some(align))
            }
            Some(TextAlignValue::MatchParent) => set(
                &mut style.text_align,
                Some(parent.text_align.matched(parent.direction)),
            ),
            Some(TextAlignValue::JustifyAll) | None => false,
        },
        "text-align-last" => match single_ident(value).map(str::to_ascii_lowercase).as_deref() {
            Some("auto") => set(&mut style.text_align_last, Some(TextAlignLast::Auto)),
            _ => match parse_text_align(value) {
                Some(TextAlignValue::Align(align) | TextAlignValue::Legacy(align, _)) => set(
                    &mut style.text_align_last,
                    Some(TextAlignLast::Align(align)),
                ),
                Some(TextAlignValue::MatchParent) => {
                    set(&mut style.text_align_last, Some(matched_last(parent)))
                }
                Some(TextAlignValue::JustifyAll) | None => false,
            },
        },
        "text-justify" => set(&mut style.text_justify, parse_text_justify(value)),
        "text-transform" => match text::parse_text_transform(value) {
            Some((case, extra)) => {
                style.text_transform = case;
                if extra != style.text().transform_extra {
                    style.text_mut().transform_extra = extra;
                }
                true
            }
            None => false,
        },
        "white-space" => set(&mut style.white_space, parse_white_space(value)),
        "white-space-collapse" => set(
            &mut style.white_space.collapse,
            parse_white_space_collapse(value),
        ),
        "text-wrap-mode" => set(&mut style.white_space.wrap, parse_text_wrap_mode(value)),
        "text-wrap" => match text::parse_text_wrap(value) {
            Some((wrap, wrap_style)) => {
                style.white_space.wrap = wrap;
                if wrap_style != style.text().text_wrap_style {
                    style.text_mut().text_wrap_style = wrap_style;
                }
                true
            }
            None => false,
        },
        "text-wrap-style" => set_text(style, text::parse_text_wrap_style(value), |t, v| {
            t.text_wrap_style = v
        }),
        "text-indent" => set_text(style, text::parse_text_indent(value, bases), |t, v| {
            t.text_indent = v
        }),
        "tab-size" => set_text(style, text::parse_tab_size(value, bases), |t, v| {
            t.tab_size = v
        }),
        "line-break" => set_text(style, text::parse_line_break(value), |t, v| {
            t.line_break = v
        }),
        "hyphens" => set_text(style, text::parse_hyphens(value), |t, v| t.hyphens = v),
        "hyphenate-character" => set_text(style, text::parse_hyphenate_character(value), |t, v| {
            t.hyphenate_character = v
        }),
        "hyphenate-limit-chars" => set_text(
            style,
            text::parse_hyphenate_limit_chars(value, bases),
            |t, v| t.hyphenate_limit_chars = v,
        ),
        "hyphenate-limit-lines" => set_text(
            style,
            text::parse_hyphenate_limit_lines(value, bases),
            |t, v| t.hyphenate_limit_lines = v,
        ),
        "text-autospace" => set_text(style, text::parse_text_autospace(value), |t, v| {
            t.text_autospace = v
        }),
        "font-palette" => set_text(style, palette::parse_font_palette(value, bases), |t, v| {
            t.font_palette = v
        }),
        "text-spacing-trim" => set_text(style, text::parse_text_spacing_trim(value), |t, v| {
            t.text_spacing_trim = v
        }),
        "ruby-position" => set_text(style, text::parse_ruby_position(value), |t, v| {
            t.ruby_position = v
        }),
        "ruby-align" => set_text(style, text::parse_ruby_align(value), |t, v| {
            t.ruby_align = v
        }),
        "math-style" => set_text(style, text::parse_math_style(value), |t, v| {
            t.math_style = v
        }),
        "hanging-punctuation" => set_text(style, text::parse_hanging_punctuation(value), |t, v| {
            t.hanging_punctuation = v
        }),
        "word-space-transform" => {
            set_text(style, text::parse_word_space_transform(value), |t, v| {
                t.word_space_transform = v
            })
        }
        "text-fit" => set_text(style, text::parse_text_fit(value), |t, v| t.text_fit = v),
        "text-group-align" => set_text(style, text::parse_text_group_align(value), |t, v| {
            t.text_group_align = v
        }),
        "unicode-bidi" => set_text(style, text::parse_unicode_bidi(value), |t, v| {
            t.unicode_bidi = v
        }),
        "overflow-wrap" => set(&mut style.overflow_wrap, parse_overflow_wrap(value)),
        "word-break" => set(&mut style.word_break, parse_word_break(value)),
        "visibility" => set(&mut style.visibility, parse_visibility(value)),
        "text-shadow" => set_decoration(style, parse_shadows(value, bases, true), |d, v| {
            d.text_shadow = v
        }),
        "letter-spacing" => match text::parse_spacing(value, bases) {
            Some(spacing) => {
                style.letter_spacing = spacing.resolve(bases.em);
                let kept = spacing.has_percent().then_some(spacing);
                if kept != style.text().letter_spacing {
                    style.text_mut().letter_spacing = kept;
                }
                true
            }
            None => false,
        },
        "word-spacing" => match text::parse_spacing(value, bases) {
            Some(spacing) => {
                style.word_spacing = spacing.resolve(bases.em);
                let kept = spacing.has_percent().then_some(spacing);
                if kept != style.text().word_spacing {
                    style.text_mut().word_spacing = kept;
                }
                true
            }
            None => false,
        },
        "width" => set(&mut style.width, parse_size(value, bases)),
        // A percentage is kept as one (M10.1); layout resolves it against
        // the containing block's height when that is definite, and treats
        // it as `auto` otherwise (CSS2 §10.5).
        "height" => set(&mut style.height, parse_size(value, bases)),
        "margin" => set(&mut style.margin, parse_margin_shorthand(value, bases)),
        "margin-top" => set_margin_edge(&mut style.margin, TOP, value, bases),
        "margin-right" => set_margin_edge(&mut style.margin, RIGHT, value, bases),
        "margin-bottom" => set_margin_edge(&mut style.margin, BOTTOM, value, bases),
        "margin-left" => set_margin_edge(&mut style.margin, LEFT, value, bases),
        "padding" => set(&mut style.padding, parse_padding_shorthand(value, bases)),
        "padding-top" => set_padding_edge(&mut style.padding, TOP, value, bases),
        "padding-right" => set_padding_edge(&mut style.padding, RIGHT, value, bases),
        "padding-bottom" => set_padding_edge(&mut style.padding, BOTTOM, value, bases),
        "padding-left" => set_padding_edge(&mut style.padding, LEFT, value, bases),
        "border-width" => set(
            &mut style.border_width,
            parse_border_width_shorthand(value, bases),
        ),
        "border-top-width" => set_border_width(style, TOP, value, bases),
        "border-right-width" => set_border_width(style, RIGHT, value, bases),
        "border-bottom-width" => set_border_width(style, BOTTOM, value, bases),
        "border-left-width" => set_border_width(style, LEFT, value, bases),
        "border-color" => match parse_1_to_4(value, |g| parse_border_color(g, bases.colors())) {
            Some(values) => {
                for (side, value) in values.into_iter().enumerate() {
                    write_border_color(style, side, value, border_color_set);
                }
                true
            }
            None => false,
        },
        "border-top-color" => set_border_color(style, TOP, value, bases.colors(), border_color_set),
        "border-right-color" => {
            set_border_color(style, RIGHT, value, bases.colors(), border_color_set)
        }
        "border-bottom-color" => {
            set_border_color(style, BOTTOM, value, bases.colors(), border_color_set)
        }
        "border-left-color" => {
            set_border_color(style, LEFT, value, bases.colors(), border_color_set)
        }
        "border-style" => set(
            &mut style.border_style,
            parse_1_to_4(value, parse_border_style),
        ),
        "border-top-style" => set_border_style(style, TOP, value),
        "border-right-style" => set_border_style(style, RIGHT, value),
        "border-bottom-style" => set_border_style(style, BOTTOM, value),
        "border-left-style" => set_border_style(style, LEFT, value),
        // It resets the border image too (Backgrounds 3 section 4.4).
        "border" => {
            let applied = apply_border_shorthand(
                style,
                &[TOP, RIGHT, BOTTOM, LEFT],
                value,
                bases,
                border_color_set,
            );
            if applied {
                style.border_image = None;
            }
            applied
        }
        "border-top" => apply_border_shorthand(style, &[TOP], value, bases, border_color_set),
        "border-right" => apply_border_shorthand(style, &[RIGHT], value, bases, border_color_set),
        "border-bottom" => apply_border_shorthand(style, &[BOTTOM], value, bases, border_color_set),
        "border-left" => apply_border_shorthand(style, &[LEFT], value, bases, border_color_set),
        "overflow" => match parse_overflow_pair(value) {
            Some((x, y)) => {
                (style.overflow_x, style.overflow_y) = (x, y);
                true
            }
            None => false,
        },
        "overflow-x" => set(&mut style.overflow_x, parse_overflow(value)),
        "overflow-y" => set(&mut style.overflow_y, parse_overflow(value)),
        // A flow-relative side is the parent's direction's (D74).
        "float" => match parse_float(value, parent.direction) {
            Some((float, flow)) => {
                style.float = float;
                style.flow_relative.float = flow;
                true
            }
            None => false,
        },
        "clear" => match parse_clear(value, parent.direction) {
            Some((clear, flow)) => {
                style.clear = clear;
                style.flow_relative.clear = flow;
                true
            }
            None => false,
        },
        "resize" => set_effects(style, parse_resize(value), |e, v| e.resize = v),
        "appearance" => set_effects(
            style,
            single_ident(value).and_then(crate::computed::Appearance::from_keyword),
            |e, v| e.appearance = v,
        ),
        // The container properties (D72).
        "container-type" => set(
            &mut style.container_type,
            crate::container::parse_container_type(value),
        ),
        "container-name" => set(
            &mut style.container_names,
            crate::container::parse_container_name(value),
        ),
        "container" => match crate::container::parse_container(value) {
            Some((names, kind)) => {
                style.container_names = names;
                style.container_type = kind;
                true
            }
            None => false,
        },
        "position" => set(&mut style.position, parse_position(value)),
        // Kept as written (M11.3): a percentage is of the containing
        // block's height or width, which only layout knows.
        "top" | "right" | "bottom" | "left" => set(
            &mut style.inset[edge_index(name)],
            parse_length_percentage_or_auto(value, bases),
        ),
        // The four insets' shorthand (CSS Positioned Layout 3 section
        // 3.1.1), one to four values as `margin` takes them.
        "inset" => set(&mut style.inset, parse_margin_shorthand(value, bases)),
        "z-index" => set(&mut style.z_index, parse_z_index(value, bases)),
        "box-sizing" => set(&mut style.box_sizing, parse_box_sizing(value)),
        "min-width" => set(&mut style.min_width, parse_size(value, bases)),
        "min-height" => set(&mut style.min_height, parse_size(value, bases)),
        "max-width" => set(&mut style.max_width, parse_max_size(value, bases)),
        "max-height" => set(&mut style.max_height, parse_max_size(value, bases)),
        "table-layout" => set(&mut style.table_layout, parse_table_layout(value)),
        "border-collapse" => set(&mut style.border_collapse, parse_border_collapse(value)),
        "border-spacing" => set(
            &mut style.border_spacing,
            parse_border_spacing(value, bases),
        ),
        "caption-side" => set(&mut style.caption_side, parse_caption_side(value)),
        "outline" => apply_outline_shorthand(style, value, bases),
        "outline-style" => set_outline(style, parse_outline_style(value), |o, v| o.style = v),
        "outline-width" => set_outline(style, parse_border_width(value, bases), |o, v| o.width = v),
        "outline-color" => {
            set_outline(style, parse_outline_color(value, bases.colors()), |o, v| {
                o.color = v
            })
        }
        "outline-offset" => set_outline(style, parse_length_px(value, bases), |o, v| o.offset = v),
        "cursor" => match parse_cursor(value) {
            Some((cursor, images)) => {
                style.cursor = cursor;
                if !images.is_empty() || !style.image().cursor_images.is_empty() {
                    style.image_mut().cursor_images = images.into();
                }
                true
            }
            None => false,
        },
        "empty-cells" => set(&mut style.empty_cells, parse_empty_cells(value)),
        "vertical-align" => set(
            &mut style.vertical_align,
            parse_vertical_align(value, bases),
        ),
        "content" => set(&mut style.content, parse_content(value, bases)),
        "quotes" => set(&mut style.quotes, parse_quotes(value)),
        "flex-direction" => set(&mut style.flex_direction, parse_flex_direction(value)),
        "flex-wrap" => set(&mut style.flex_wrap, parse_flex_wrap(value)),
        "flex-flow" => parse_flex_flow(value)
            .map(|(d, w)| {
                style.flex_direction = d;
                style.flex_wrap = w;
            })
            .is_some(),
        "order" => set(&mut style.order, parse_order(value, bases)),
        "flex-grow" => set(&mut style.flex_grow, parse_flex_factor(value, bases)),
        "flex-shrink" => set(&mut style.flex_shrink, parse_flex_factor(value, bases)),
        "flex-basis" => set(&mut style.flex_basis, parse_flex_basis(value, bases)),
        "flex" => parse_flex(value, bases)
            .map(|(grow, shrink, basis)| {
                style.flex_grow = grow;
                style.flex_shrink = shrink;
                style.flex_basis = basis;
            })
            .is_some(),
        "justify-content" | "align-content" => {
            let justify = name == "justify-content";
            let words = idents(&split_components(value));
            let slot = if justify {
                &mut style.justify_content
            } else {
                &mut style.align_content
            };
            set(slot, words.and_then(|w| content_alignment(&w, justify)))
        }
        "align-items" | "align-self" => {
            let items = name == "align-items";
            let words = idents(&split_components(value));
            let slot = if items {
                &mut style.align_items
            } else {
                &mut style.align_self
            };
            set(slot, words.and_then(|w| self_alignment(&w, !items)))
        }
        "place-content" => parse_place_content(value)
            .map(|(align, justify)| {
                style.align_content = align;
                style.justify_content = justify;
            })
            .is_some(),
        "justify-items" => {
            let parent_items = parent.grid().justify_items;
            let words = idents(&split_components(value));
            set_grid(
                style,
                words.and_then(|w| grid::justify_items(&w, parent_items)),
                |g, v| g.justify_items = v,
            )
        }
        "justify-self" => {
            let words = idents(&split_components(value));
            set_grid(
                style,
                words.and_then(|w| grid::justify_self(&w, true)),
                |g, v| g.justify_self = v,
            )
        }
        "reading-flow" => set_grid(style, containment::parse_reading_flow(value), |g, v| {
            g.reading_flow = v
        }),
        "reading-order" => set_grid(
            style,
            containment::parse_reading_order(value, bases),
            |g, v| g.reading_order = v,
        ),
        "contain" => set_containment(style, containment::parse_contain(value), |c, v| {
            c.contain = v
        }),
        "content-visibility" => set_containment(
            style,
            containment::parse_content_visibility(value),
            |c, v| c.content_visibility = v,
        ),
        "contain-intrinsic-size" => set_containment(
            style,
            containment::parse_contain_intrinsic_size(value, bases),
            |c, v| c.intrinsic = v,
        ),
        // The logical longhands map as horizontal text maps them, the
        // only writing mode the engine lays out.
        "contain-intrinsic-width" | "contain-intrinsic-inline-size" => set_containment(
            style,
            containment::parse_contain_intrinsic_axis(value, bases),
            |c, v| c.intrinsic[0] = v,
        ),
        "contain-intrinsic-height" | "contain-intrinsic-block-size" => set_containment(
            style,
            containment::parse_contain_intrinsic_axis(value, bases),
            |c, v| c.intrinsic[1] = v,
        ),
        "place-items" => match parse_place_items(value, parent.grid().justify_items) {
            Some((align, justify)) => {
                style.align_items = align;
                set_grid(style, Some(justify), |g, v| g.justify_items = v)
            }
            None => false,
        },
        "place-self" => match parse_place_self(value) {
            Some((align, justify)) => {
                style.align_self = align;
                set_grid(style, Some(justify), |g, v| g.justify_self = v)
            }
            None => false,
        },
        "grid-template-columns" => set_grid(style, grid::parse_template(value, bases), |g, v| {
            g.template_columns = v
        }),
        "grid-template-rows" => set_grid(style, grid::parse_template(value, bases), |g, v| {
            g.template_rows = v
        }),
        "grid-template-areas" => set_grid(style, grid::parse_template_areas(value), |g, v| {
            g.template_areas = v
        }),
        "grid-template" => set_grid(
            style,
            grid::parse_grid_template(value, bases),
            |g, (rows, columns, areas)| {
                g.template_rows = rows;
                g.template_columns = columns;
                g.template_areas = areas;
            },
        ),
        "grid-auto-columns" => set_grid(style, grid::parse_auto_tracks(value, bases), |g, v| {
            g.auto_columns = v
        }),
        "grid-auto-rows" => set_grid(style, grid::parse_auto_tracks(value, bases), |g, v| {
            g.auto_rows = v
        }),
        "grid-auto-flow" => set_grid(style, grid::parse_auto_flow(value), |g, v| g.auto_flow = v),
        "grid" => set_grid(style, grid::parse_grid(value, bases), |g, v| {
            (g.template_rows, g.template_columns, g.template_areas) = v.template;
            g.auto_flow = v.auto_flow;
            g.auto_rows = v.auto_rows;
            g.auto_columns = v.auto_columns;
        }),
        "grid-row-start" => set_grid(style, grid::parse_grid_line(value, bases), |g, v| {
            g.row_start = v
        }),
        "grid-row-end" => set_grid(style, grid::parse_grid_line(value, bases), |g, v| {
            g.row_end = v
        }),
        "grid-column-start" => set_grid(style, grid::parse_grid_line(value, bases), |g, v| {
            g.column_start = v
        }),
        "grid-column-end" => set_grid(style, grid::parse_grid_line(value, bases), |g, v| {
            g.column_end = v
        }),
        "grid-row" => set_grid(
            style,
            grid::parse_grid_lines(value, bases),
            |g, (start, end)| {
                g.row_start = start;
                g.row_end = end;
            },
        ),
        "grid-column" => set_grid(
            style,
            grid::parse_grid_lines(value, bases),
            |g, (start, end)| {
                g.column_start = start;
                g.column_end = end;
            },
        ),
        "grid-area" => set_grid(
            style,
            grid::parse_grid_area(value, bases),
            |g, [rs, cs, re, ce]| {
                g.row_start = rs;
                g.column_start = cs;
                g.row_end = re;
                g.column_end = ce;
            },
        ),
        "row-gap" => set(&mut style.row_gap, parse_gap(value, bases)),
        "column-gap" => set(&mut style.column_gap, parse_gap(value, bases)),
        "gap" => parse_gap_shorthand(value, bases)
            .map(|(row, column)| {
                style.row_gap = row;
                style.column_gap = column;
            })
            .is_some(),
        "transform" => set_effects(style, effects::parse_transform(value, bases), |e, v| {
            e.transform = v
        }),
        "transform-origin" => set_effects(
            style,
            effects::parse_transform_origin(value, bases),
            |e, v| e.transform_origin = v,
        ),
        "transform-box" => set_effects(style, effects::parse_transform_box(value), |e, v| {
            e.transform_box = v
        }),
        "translate" => set_effects(style, effects::parse_translate(value, bases), |e, v| {
            e.translate = v
        }),
        "rotate" => set_effects(style, effects::parse_rotate(value, bases), |e, v| {
            e.rotate = v
        }),
        "scale" => set_effects(style, effects::parse_scale(value, bases), |e, v| {
            e.scale = v
        }),
        "transform-style" => set_effects(style, effects::parse_transform_style(value), |e, v| {
            e.transform_style = v
        }),
        "perspective" => set_effects(style, effects::parse_perspective(value, bases), |e, v| {
            e.perspective = v
        }),
        "perspective-origin" => set_effects(
            style,
            effects::parse_perspective_origin(value, bases),
            |e, v| e.perspective_origin = v,
        ),
        "backface-visibility" => {
            set_effects(style, effects::parse_backface_visibility(value), |e, v| {
                e.backface_visibility = v
            })
        }
        "overlay" => set_effects(style, effects::parse_overlay(value), |e, v| e.overlay = v),
        // `scroll-behavior` (CSSOM View section 13.1): `auto | smooth`.
        "scroll-behavior" => {
            let smooth = match single_ident(value).map(str::to_ascii_lowercase).as_deref() {
                Some("auto") => Some(false),
                Some("smooth") => Some(true),
                _ => None,
            };
            set_effects(style, smooth, |e, v| e.scroll_behavior_smooth = v)
        }
        // CSS UI 4 section 7.1 and HTML section 6.14 (gap batch H).
        "interactivity" => set(
            &mut style.interactivity_inert,
            top_layer::parse_interactivity(value),
        ),
        "pointer-events" => set(
            &mut style.pointer_events,
            top_layer::parse_pointer_events(value),
        ),
        "interest-delay-start" => set_effects(
            style,
            top_layer::parse_interest_delay(value, bases),
            |e, v| e.interest_delay[0] = v,
        ),
        "interest-delay-end" => set_effects(
            style,
            top_layer::parse_interest_delay(value, bases),
            |e, v| e.interest_delay[1] = v,
        ),
        "interest-delay" => set_effects(
            style,
            top_layer::parse_interest_delay_shorthand(value, bases),
            |e, v| e.interest_delay = v,
        ),
        "scroll-snap-type" => set_effects(style, parse_scroll_snap_type(value), |e, v| {
            e.scroll_snap_type = v
        }),
        "scroll-snap-align" => set_effects(style, parse_scroll_snap_align(value), |e, v| {
            e.scroll_snap_align = v
        }),
        "scroll-snap-stop" => {
            let always = match single_ident(value).map(str::to_ascii_lowercase).as_deref() {
                Some("normal") => Some(false),
                Some("always") => Some(true),
                _ => None,
            };
            set_effects(style, always, |e, v| e.scroll_snap_stop_always = v)
        }
        "scroll-initial-target" => {
            let nearest = match single_ident(value).map(str::to_ascii_lowercase).as_deref() {
                Some("none") => Some(false),
                Some("nearest") => Some(true),
                _ => None,
            };
            set_effects(style, nearest, |e, v| e.scroll_initial_target = v)
        }
        // Scroll Snap 1 section 6: lengths, and `auto` or lengths and
        // percentages not below zero.
        "scroll-margin" => {
            let values: Option<Vec<f32>> = split_components(value)
                .iter()
                .map(|g| parse_length_px(g, bases))
                .collect();
            set_effects(style, expand_1_to_4(&values.unwrap_or_default()), |e, v| {
                e.scroll_margin = v
            })
        }
        "scroll-padding" => {
            let values: Option<Vec<LengthPercentageAuto>> = split_components(value)
                .iter()
                .map(|g| parse_scroll_padding(g, bases))
                .collect();
            set_effects(style, expand_1_to_4(&values.unwrap_or_default()), |e, v| {
                e.scroll_padding = v
            })
        }
        "scroll-margin-top"
        | "scroll-margin-right"
        | "scroll-margin-bottom"
        | "scroll-margin-left" => {
            let side = side_of(name);
            set_effects(style, parse_length_px(value, bases), |e, v| {
                e.scroll_margin[side] = v
            })
        }
        "scroll-padding-top"
        | "scroll-padding-right"
        | "scroll-padding-bottom"
        | "scroll-padding-left" => {
            let side = side_of(name);
            set_effects(style, parse_scroll_padding(value, bases), |e, v| {
                e.scroll_padding[side] = v
            })
        }
        "box-shadow" => set_effects(style, parse_shadows(value, bases, false), |e, v| {
            e.box_shadow = v
        }),
        "filter" => set_effects(style, parse_filter(value, bases), |e, v| e.filter = v),
        "opacity" => set_effects(style, effects::parse_opacity(value, bases), |e, v| {
            e.opacity = v
        }),
        "will-change" => set_effects(style, effects::parse_will_change(value), |e, v| {
            e.will_change = v
        }),
        "transition-property" => {
            set_motion(style, motion::parse_transition_property(value), |m, v| {
                m.transition_property = v
            })
        }
        "transition-duration" => {
            set_motion(style, motion::parse_durations(value, bases), |m, v| {
                m.transition_duration = v
            })
        }
        "transition-timing-function" => {
            set_motion(style, motion::parse_easings(value, bases), |m, v| {
                m.transition_timing_function = v
            })
        }
        "transition-delay" => set_motion(style, motion::parse_delays(value, bases), |m, v| {
            m.transition_delay = v
        }),
        "transition-behavior" => set_motion(style, motion::parse_behaviors(value), |m, v| {
            m.transition_behavior = v
        }),
        "transition" => set_motion(style, motion::parse_transition(value, bases), |m, v| {
            m.transition_property = v.property;
            m.transition_duration = v.duration;
            m.transition_timing_function = v.timing_function;
            m.transition_delay = v.delay;
            m.transition_behavior = v.behavior;
        }),
        "animation-name" => set_motion(style, motion::parse_animation_names(value), |m, v| {
            m.animation_name = v
        }),
        "animation-duration" => set_motion(
            style,
            motion::parse_animation_durations(value, bases),
            |m, v| m.animation_duration = v,
        ),
        "animation-timing-function" => {
            set_motion(style, motion::parse_easings(value, bases), |m, v| {
                m.animation_timing_function = v
            })
        }
        "animation-iteration-count" => set_motion(
            style,
            motion::parse_iteration_counts(value, bases),
            |m, v| m.animation_iteration_count = v,
        ),
        "animation-direction" => set_motion(style, motion::parse_directions(value), |m, v| {
            m.animation_direction = v
        }),
        "animation-play-state" => set_motion(style, motion::parse_play_states(value), |m, v| {
            m.animation_play_state = v
        }),
        "animation-delay" => set_motion(style, motion::parse_delays(value, bases), |m, v| {
            m.animation_delay = v
        }),
        "animation-fill-mode" => set_motion(style, motion::parse_fill_modes(value), |m, v| {
            m.animation_fill_mode = v
        }),
        "animation-composition" => set_motion(style, motion::parse_compositions(value), |m, v| {
            m.animation_composition = v
        }),
        "animation" => set_motion(style, motion::parse_animation(value, bases), |m, v| {
            m.animation_name = v.name;
            m.animation_duration = v.duration;
            m.animation_timing_function = v.timing_function;
            m.animation_iteration_count = v.iteration_count;
            m.animation_direction = v.direction;
            m.animation_play_state = v.play_state;
            m.animation_delay = v.delay;
            m.animation_fill_mode = v.fill_mode;
            // The reset-only longhands of the shorthand (Animations 2,
            // Scroll-driven Animations 1 section 4).
            m.animation_composition = vec![crate::computed::Composite::Replace];
            m.animation_timeline = vec![crate::computed::Timeline::Auto];
            m.animation_range_start = vec![crate::computed::Edge::Normal];
            m.animation_range_end = vec![crate::computed::Edge::Normal];
        }),
        "break-before" => {
            set_fragmentation(style, fragmentation::parse_break_between(value), |f, v| {
                f.break_before = v
            })
        }
        "break-after" => {
            set_fragmentation(style, fragmentation::parse_break_between(value), |f, v| {
                f.break_after = v
            })
        }
        "break-inside" => {
            set_fragmentation(style, fragmentation::parse_break_inside(value), |f, v| {
                f.break_inside = v
            })
        }
        "page-break-before" => set_fragmentation(
            style,
            fragmentation::parse_page_break_between(value),
            |f, v| f.break_before = v,
        ),
        "page-break-after" => set_fragmentation(
            style,
            fragmentation::parse_page_break_between(value),
            |f, v| f.break_after = v,
        ),
        "page-break-inside" => set_fragmentation(
            style,
            fragmentation::parse_page_break_inside(value),
            |f, v| f.break_inside = v,
        ),
        "orphans" => set_fragmentation(
            style,
            fragmentation::parse_line_count(value, bases),
            |f, v| f.orphans = v,
        ),
        "widows" => set_fragmentation(
            style,
            fragmentation::parse_line_count(value, bases),
            |f, v| f.widows = v,
        ),
        "print-color-adjust" => set_fragmentation(
            style,
            fragmentation::parse_print_color_adjust(value),
            |f, v| f.print_color_adjust = v,
        ),
        "counter-reset" => set_lists(style, lists::parse_counter_reset(value, bases), |l, v| {
            l.counter_reset = v
        }),
        "counter-increment" => set_lists(
            style,
            lists::parse_counter_changes(value, bases, 1),
            |l, v| l.counter_increment = v,
        ),
        "counter-set" => set_lists(
            style,
            lists::parse_counter_changes(value, bases, 0),
            |l, v| l.counter_set = v,
        ),
        "list-style-type" => set_lists(style, lists::parse_list_style_type(value), |l, v| {
            l.list_style_type = v
        }),
        "list-style-position" => {
            set_lists(style, lists::parse_list_style_position(value), |l, v| {
                l.list_style_position = v
            })
        }
        "list-style-image" => set_lists(
            style,
            lists::parse_list_style_image(value, bases),
            |l, v| l.list_style_image = v,
        ),
        "list-style" => set_lists(
            style,
            lists::parse_list_style(value, bases),
            |l, (position, image, kind)| {
                l.list_style_position = position;
                l.list_style_image = image;
                l.list_style_type = kind;
            },
        ),
        "aspect-ratio" => set_image(style, images::parse_aspect_ratio(value, bases), |i, v| {
            i.aspect_ratio = v
        }),
        "object-fit" => set_image(style, images::parse_object_fit(value), |i, v| {
            i.object_fit = v
        }),
        "object-position" => set_image(
            style,
            images::parse_object_position(value, bases),
            |i, v| i.object_position = v,
        ),
        "image-orientation" => set_image(style, images::parse_image_orientation(value), |i, v| {
            i.image_orientation = v
        }),
        "image-rendering" => set_image(style, images::parse_image_rendering(value), |i, v| {
            i.image_rendering = v
        }),
        "text-decoration-line" => {
            set_decoration(style, decoration::parse_line(value), |d, v| d.line = v)
        }
        "text-decoration-style" => {
            set_decoration(style, decoration::parse_style(value), |d, v| d.style = v)
        }
        "text-decoration-color" => set_decoration(
            style,
            decoration::parse_color_or_current(value, bases.colors()),
            |d, v| d.color = v,
        ),
        "text-decoration-thickness" => {
            set_decoration(style, decoration::parse_thickness(value, bases), |d, v| {
                d.thickness = v
            })
        }
        "text-decoration" => set_decoration(
            style,
            decoration::parse_shorthand(value, bases),
            |d, (line, style, color, thickness)| {
                d.line = line;
                d.style = style;
                d.color = color;
                d.thickness = thickness;
            },
        ),
        "text-underline-offset" => set_decoration(
            style,
            decoration::parse_underline_offset(value, bases),
            |d, v| d.underline_offset = v,
        ),
        "text-underline-position" => set_decoration(
            style,
            decoration::parse_underline_position(value),
            |d, v| d.underline_position = v,
        ),
        // SVG's (M32); else not in the property set, or not a real
        // property: ignored, never an error.
        _ => svg::apply(style, name, value, bases).unwrap_or(false),
    }
}

/// A `font-size` (CSS2 §15.7): a non-negative length, or a percentage
/// of the parent's font size, which is `bases.em` here; or a keyword
/// (D60): an absolute size, the reference browser's at a `medium` of
/// 16px, or `larger` and `smaller`, the parent's by 1.2.
fn parse_font_size(value: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if let Some(word) = single_ident(value) {
        return Some(match word.to_ascii_lowercase().as_str() {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "xxx-large" => 48.0,
            "larger" => bases.em * 1.2,
            "smaller" => bases.em / 1.2,
            _ => return None,
        });
    }
    if let Some(calc::Computed::Value(px)) = math(
        value,
        Want::LengthPercentage,
        bases,
        Some(f64::from(bases.em)),
    ) {
        return Some(Range::NonNegative.clamp(px));
    }
    if let Some(Token::Percentage { value }) = single_token(value) {
        let px = *value as f32 / 100.0 * bases.em;
        return (px.is_finite() && px >= 0.0).then(|| px.min(MAX_LENGTH_PX));
    }
    parse_non_negative_length_px(value, bases)
}

/// Writes `v` into `slot` when there is one: whether a declaration's
/// value parsed, for [`apply_declaration`]'s answer.
fn set<T>(slot: &mut T, v: Option<T>) -> bool {
    v.map(|v| *slot = v).is_some()
}

/// [`set`] for the font properties behind [`ComputedStyle::font`]
/// (D53).
fn set_font<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut FontProps, T),
) -> bool {
    v.map(|v| update(style.font_mut(), v)).is_some()
}

/// [`set`] for the grid properties (M18), which live behind
/// [`ComputedStyle::grid`]: `update` writes `v`, when there is one, into
/// the style's own copy of them.
fn set_grid<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut GridStyle, T),
) -> bool {
    v.map(|v| update(style.grid_mut(), v)).is_some()
}

/// Copies grid properties from `src`, a parent's style or the initial
/// one, by `copy`: nothing to do when neither has any set, both being
/// the initial values.
fn copy_grid(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut GridStyle, &GridStyle),
) {
    if style.grid.is_none() && src.grid.is_none() {
        return;
    }
    let src = src.grid().clone();
    copy(style.grid_mut(), &src);
}

/// [`set`] for the transform, opacity and will-change properties (M19),
/// which live behind [`ComputedStyle::effects`].
fn set_effects<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Effects, T),
) -> bool {
    v.map(|v| update(style.effects_mut(), v)).is_some()
}

/// [`set`] for the containment properties (gap batch H), which live
/// behind [`ComputedStyle::containment`].
fn set_containment<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Containment, T),
) -> bool {
    dom::hook!("style::set_containment");
    v.map(|v| update(style.containment_mut(), v)).is_some()
}

/// [`copy_grid`] for the containment properties.
fn copy_containment(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Containment, &Containment),
) {
    dom::hook!("style::copy_containment");
    if style.containment.is_none() && src.containment.is_none() {
        return;
    }
    let src = src.containment().clone();
    copy(style.containment_mut(), &src);
}

/// [`set`] for the break properties, `orphans`, `widows` and
/// `print-color-adjust` (M20), which live behind
/// [`ComputedStyle::fragmentation`].
fn set_fragmentation<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Fragmentation, T),
) -> bool {
    v.map(|v| update(style.fragmentation_mut(), v)).is_some()
}

/// [`copy_grid`] for the fragmentation properties.
fn copy_fragmentation(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Fragmentation, &Fragmentation),
) {
    if style.fragmentation.is_none() && src.fragmentation.is_none() {
        return;
    }
    let src = src.fragmentation().clone();
    copy(style.fragmentation_mut(), &src);
}

/// [`set_grid`] for the counter and list properties.
fn set_lists<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Lists, T),
) -> bool {
    v.map(|v| update(style.lists_mut(), v)).is_some()
}

/// [`set_grid`] for the background properties (M23.3).
fn set_background<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Backgrounds, T),
) -> bool {
    v.map(|v| update(style.background_mut(), v)).is_some()
}

/// [`set_grid`] for the border image properties (M23.3).
fn set_border_image<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut BorderImage, T),
) -> bool {
    v.map(|v| update(style.border_image_mut(), v)).is_some()
}

/// [`set_grid`] for the image properties (M23.2).
fn set_image<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut ImageStyle, T),
) -> bool {
    v.map(|v| update(style.image_mut(), v)).is_some()
}

/// [`set_grid`] for the text decoration properties.
fn set_decoration<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Decoration, T),
) -> bool {
    v.map(|v| update(style.decoration_mut(), v)).is_some()
}

/// [`set`] for the text properties behind [`ComputedStyle::text`].
fn set_text<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut crate::computed::TextProps, T),
) -> bool {
    v.map(|v| update(style.text_mut(), v)).is_some()
}

/// [`copy_grid`] for the text properties behind [`ComputedStyle::text`].
fn copy_text(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut crate::computed::TextProps, &crate::computed::TextProps),
) {
    if style.text.is_none() && src.text.is_none() {
        return;
    }
    let src = src.text().clone();
    copy(style.text_mut(), &src);
}

/// [`set_grid`] for the transition and animation properties (M22).
fn set_motion<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Motion, T),
) -> bool {
    v.map(|v| update(style.motion_mut(), v)).is_some()
}

/// [`copy_grid`] for the transition and animation properties.
fn copy_motion(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Motion, &Motion),
) {
    if style.motion.is_none() && src.motion.is_none() {
        return;
    }
    let src = src.motion().clone();
    copy(style.motion_mut(), &src);
}

/// [`copy_grid`] for the text decoration properties.
fn copy_decoration(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Decoration, &Decoration),
) {
    if style.decoration.is_none() && src.decoration.is_none() {
        return;
    }
    let src = src.decoration().clone();
    copy(style.decoration_mut(), &src);
}

/// [`copy_grid`] for the counter and list properties.
fn copy_lists(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Lists, &Lists),
) {
    if style.lists.is_none() && src.lists.is_none() {
        return;
    }
    let src = src.lists().clone();
    copy(style.lists_mut(), &src);
}

/// [`copy_grid`] for the background properties (M23.3).
fn copy_background(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Backgrounds, &Backgrounds),
) {
    if style.background.is_none() && src.background.is_none() {
        return;
    }
    let src = src.background().clone();
    copy(style.background_mut(), &src);
}

/// [`copy_grid`] for the border image properties (M23.3).
fn copy_border_image(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut BorderImage, &BorderImage),
) {
    if style.border_image.is_none() && src.border_image.is_none() {
        return;
    }
    let src = src.border_image().clone();
    copy(style.border_image_mut(), &src);
}

/// [`copy_grid`] for the image properties (M23.2).
fn copy_image(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut ImageStyle, &ImageStyle),
) {
    if style.image.is_none() && src.image.is_none() {
        return;
    }
    let src = src.image().clone();
    copy(style.image_mut(), &src);
}

/// [`copy_grid`] for the effects properties.
/// [`set`] for the outline properties (M26.4), which live behind
/// [`ComputedStyle::outline`].
fn set_outline<T>(
    style: &mut ComputedStyle,
    v: Option<T>,
    update: impl FnOnce(&mut Outline, T),
) -> bool {
    v.map(|v| update(style.outline_mut(), v)).is_some()
}

/// Copies an outline property from `src`, leaving the group unmade when
/// neither has one.
fn copy_outline(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Outline, &Outline),
) {
    if style.outline.is_none() && src.outline.is_none() {
        return;
    }
    let src = src.outline().clone();
    copy(style.outline_mut(), &src);
}

fn copy_effects(
    style: &mut ComputedStyle,
    src: &ComputedStyle,
    copy: impl FnOnce(&mut Effects, &Effects),
) {
    if style.effects.is_none() && src.effects.is_none() {
        return;
    }
    let src = src.effects().clone();
    copy(style.effects_mut(), &src);
}

/// Writes a `border-*-width` longhand: a width keyword or a length; a
/// value that fails to parse — a negative one included, which CSS2
/// §8.5.1 forbids — leaves that side untouched, exactly like any other
/// declaration this engine can't make sense of.
fn set_border_width(
    style: &mut ComputedStyle,
    side: usize,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> bool {
    set(
        &mut style.border_width[side],
        parse_border_width(tokens, bases),
    )
}

/// Writes a `border-*-color` longhand.
fn set_border_color(
    style: &mut ComputedStyle,
    side: usize,
    tokens: &[Token],
    colors: Colors,
    color_set: &mut [bool; 4],
) -> bool {
    let Some(value) = parse_border_color(tokens, colors) else {
        return false;
    };
    write_border_color(style, side, value, color_set);
    true
}

/// A `border-*-color` value: a colour, or `None` for `currentcolor`
/// (M33), which the cascade resolves once `color` is final.
fn parse_border_color(tokens: &[Token], colors: Colors) -> Option<Option<Color>> {
    match single_ident(tokens) {
        Some(ident) if ident.eq_ignore_ascii_case("currentcolor") => Some(None),
        _ => color_value(tokens, colors).map(Some),
    }
}

/// Writes one side's `border-*-color`, `None` being `currentcolor`.
fn write_border_color(
    style: &mut ComputedStyle,
    side: usize,
    value: Option<Color>,
    color_set: &mut [bool; 4],
) {
    if let Some(color) = value {
        style.border_color[side] = color;
    }
    color_set[side] = value.is_some();
}

/// Writes a `border-*-style` longhand.
fn set_border_style(style: &mut ComputedStyle, side: usize, tokens: &[Token]) -> bool {
    set(&mut style.border_style[side], parse_border_style(tokens))
}

/// A `<line-width>` (CSS2 §8.5.1): `thin`, `medium`, `thick`, or a
/// non-negative length.
fn parse_border_width(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if let Some(ident) = single_ident(tokens) {
        return match ident.to_ascii_lowercase().as_str() {
            "thin" => Some(1.0),
            "medium" => Some(3.0),
            "thick" => Some(5.0),
            _ => None,
        };
    }
    parse_non_negative_length_px(tokens, bases)
}

/// The `border-width` shorthand: one to four `<line-width>`s.
fn parse_border_width_shorthand(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Edges> {
    parse_1_to_4(tokens, |g| parse_border_width(g, bases))
}

/// One to four space-separated values parsed by `one`, expanded to the
/// four sides per CSS2 §8.3's rule; one bad value drops the whole
/// shorthand.
fn parse_1_to_4<T: Copy>(tokens: &[Token], one: impl Fn(&[Token]) -> Option<T>) -> Option<[T; 4]> {
    let groups = split_components(tokens);
    let values: Option<Vec<T>> = groups.iter().map(|g| one(g)).collect();
    expand_1_to_4(&values?)
}

/// Like [`set_edge`], for a `padding-*` longhand, which also takes a
/// percentage (CSS2 §8.4: never negative either).
fn set_padding_edge(
    edges: &mut PaddingEdges,
    which: usize,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> bool {
    set(
        &mut edges[which],
        parse_non_negative_length_percentage(tokens, bases),
    )
}

/// The CSS-wide keywords (css-cascade-5 §7.3), which every property
/// accepts in place of its own value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CssWide {
    Inherit,
    Initial,
    Unset,
}

/// The CSS-wide keyword `tokens` are, alone but for white space.
pub(crate) fn css_wide_keyword(tokens: &[Token]) -> Option<CssWide> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "inherit" => Some(CssWide::Inherit),
        "initial" => Some(CssWide::Initial),
        "unset" => Some(CssWide::Unset),
        _ => None,
    }
}

/// The longhands `all` sets: every one the engine computes but
/// `direction` and `unicode-bidi` (Cascade 5 section 3.1), the
/// flow-relative ones as the physical ones they name (D74).
pub fn all_longhands() -> impl Iterator<Item = &'static str> {
    crate::serialize::COMPUTED_PROPERTIES
        .iter()
        .copied()
        .filter(|name| !matches!(*name, "direction" | "unicode-bidi"))
        .filter(|name| !LOGICAL_LONGHANDS.contains(name))
}

/// Whether `name` is an inherited property (css-cascade-5 §7.1 and
/// each property's own definition): what `unset` means for it.
pub(crate) fn is_inherited(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "color-scheme"
            | "font-size"
            | "font-family"
            | "font"
            | "font-weight"
            | "font-style"
            | "font-stretch"
            | "font-variant"
            | "font-variant-caps"
            | "font-variant-ligatures"
            | "font-variant-alternates"
            | "font-variant-numeric"
            | "font-variant-east-asian"
            | "font-variant-position"
            | "font-variant-emoji"
            | "font-feature-settings"
            | "font-kerning"
            | "line-height"
            | "text-align"
            | "text-transform"
            | "white-space"
            | "white-space-collapse"
            | "text-wrap-mode"
            | "text-wrap"
            | "text-wrap-style"
            | "text-indent"
            | "tab-size"
            | "line-break"
            | "hyphens"
            | "hyphenate-character"
            | "hyphenate-limit-chars"
            | "hyphenate-limit-lines"
            | "text-autospace"
            | "font-palette"
            | "text-spacing-trim"
            | "ruby-position"
            | "ruby-align"
            | "math-style"
            | "hanging-punctuation"
            | "word-space-transform"
            | "text-fit"
            | "overflow-wrap"
            | "word-break"
            | "visibility"
            | "interactivity"
            | "pointer-events"
            | "text-shadow"
            | "letter-spacing"
            | "word-spacing"
            | "border-collapse"
            | "border-spacing"
            | "caption-side"
            | "cursor"
            | "empty-cells"
            | "quotes"
            | "direction"
            | "orphans"
            | "widows"
            | "print-color-adjust"
            | "list-style"
            | "list-style-type"
            | "list-style-position"
            | "list-style-image"
            | "text-underline-offset"
            | "text-underline-position"
            | "text-align-all"
            | "text-align-last"
            | "text-justify"
            | "image-orientation"
            | "image-rendering"
    ) || svg::is_inherited(name)
}

/// Applies `name: <keyword>`: the parent's computed value for
/// `inherit`, the initial value for `initial`, whichever the property's
/// inheritance says for `unset`. `font-size` is handled in the first
/// pass of [`compute_style`] and ignored here. Says whether `name` is a
/// property this engine knows.
fn apply_css_wide(
    style: &mut ComputedStyle,
    name: &str,
    keyword: CssWide,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) -> bool {
    let initial = ComputedStyle::default();
    let from_parent = match keyword {
        CssWide::Inherit => true,
        CssWide::Initial => false,
        CssWide::Unset => is_inherited(name),
    };
    let src = if from_parent { parent } else { &initial };
    // A flow-relative property's physical ones, by the element's
    // direction (D74).
    if let Some(mapped) = logical::map(name, style.direction) {
        return match mapped {
            logical::Mapped::One(physical) => {
                apply_css_wide(style, &physical, keyword, parent, border_color_set)
            }
            logical::Mapped::Pair([start, end]) | logical::Mapped::Both([start, end]) => {
                apply_css_wide(style, &start, keyword, parent, border_color_set)
                    && apply_css_wide(style, &end, keyword, parent, border_color_set)
            }
        };
    }
    if timelines::is_timeline_property(name) {
        timelines::copy(style, src, name);
        return true;
    }
    match name {
        // Every longhand but `direction` and `unicode-bidi` (Cascade 5
        // section 3.1).
        // `color` and `font-size` are the cascade's first passes'.
        "all" => {
            for longhand in all_longhands().filter(|l| !matches!(*l, "color" | "font-size")) {
                apply_css_wide(style, longhand, keyword, parent, border_color_set);
            }
        }
        "color" => style.color = src.color,
        "color-scheme" => style.color_scheme = src.color_scheme.clone(),
        "direction" => style.direction = src.direction,
        "background-color" => {
            style.background_color = src.background_color;
            keep_color_mix(style, src.background().color_mix.clone());
        }
        "background" => {
            style.background_color = src.background_color;
            style.background = src.background.clone();
        }
        "background-image" => copy_background(style, src, |b, s| b.image = s.image.clone()),
        "background-repeat" => copy_background(style, src, |b, s| b.repeat = s.repeat.clone()),
        "background-position" => copy_background(style, src, |b, s| {
            b.position_x = s.position_x.clone();
            b.position_y = s.position_y.clone();
        }),
        "background-position-x" => {
            copy_background(style, src, |b, s| b.position_x = s.position_x.clone())
        }
        "background-position-y" => {
            copy_background(style, src, |b, s| b.position_y = s.position_y.clone())
        }
        "background-size" => copy_background(style, src, |b, s| b.size = s.size.clone()),
        "background-origin" => copy_background(style, src, |b, s| b.origin = s.origin.clone()),
        "background-clip" => copy_background(style, src, |b, s| b.clip = s.clip.clone()),
        "background-attachment" => {
            copy_background(style, src, |b, s| b.attachment = s.attachment.clone())
        }
        "border-image" => style.border_image = src.border_image.clone(),
        "border-radius" => style.radii = src.radii.clone(),
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius" => {
            let corner = corner_of(name);
            style.radii_mut().0[corner] = src.radii().0[corner].clone();
        }
        "border-image-source" => copy_border_image(style, src, |b, s| b.source = s.source.clone()),
        "border-image-slice" => copy_border_image(style, src, |b, s| {
            b.slice = s.slice;
            b.fill = s.fill;
        }),
        "border-image-width" => copy_border_image(style, src, |b, s| b.width = s.width.clone()),
        "border-image-outset" => copy_border_image(style, src, |b, s| b.outset = s.outset),
        "border-image-repeat" => copy_border_image(style, src, |b, s| b.repeat = s.repeat),
        "font-size" => {} // the first pass, see `compute_style`
        "font-family" => style.font_mut().family = src.font().family.clone(),
        // Its size is the first pass's.
        "font" => {
            style.font = src.font.clone();
        }
        "font-weight" => style.font_mut().weight = src.font().weight,
        "font-style" => style.font_mut().style = src.font().style,
        "font-stretch" => style.font_mut().stretch = src.font().stretch,
        "font-variant" => style.font_mut().variant = src.font().variant.clone(),
        "font-variant-ligatures"
        | "font-variant-caps"
        | "font-variant-alternates"
        | "font-variant-numeric"
        | "font-variant-east-asian"
        | "font-variant-position"
        | "font-variant-emoji" => {
            let value = fonts::variant_value(&src.font().variant, name).map(str::to_owned);
            let font = style.font_mut();
            if let (Some(value), Some(field)) = (
                value,
                fonts::variant_field(Arc::make_mut(&mut font.variant), name),
            ) {
                *field = value;
            }
        }
        "font-feature-settings" => style.font_mut().features = src.font().features.clone(),
        "font-kerning" => style.font_mut().kerning = src.font().kerning,
        "line-height" => style.font_mut().line_height = src.font().line_height,
        "display" => {
            style.display = src.display;
            style.flow_root = src.flow_root;
        }
        "text-align" => {
            style.text_align = src.text_align;
            style.text_align_last = src.text_align_last;
            style.legacy_align = src.legacy_align;
        }
        "text-align-all" => style.text_align = src.text_align,
        "text-align-last" => style.text_align_last = src.text_align_last,
        "text-justify" => style.text_justify = src.text_justify,
        "text-transform" => {
            style.text_transform = src.text_transform;
            copy_text(style, src, |t, s| t.transform_extra = s.transform_extra);
        }
        "white-space" => style.white_space = src.white_space,
        "white-space-collapse" => style.white_space.collapse = src.white_space.collapse,
        "text-wrap-mode" => style.white_space.wrap = src.white_space.wrap,
        "text-wrap" => {
            style.white_space.wrap = src.white_space.wrap;
            copy_text(style, src, |t, s| t.text_wrap_style = s.text_wrap_style);
        }
        "text-wrap-style" => copy_text(style, src, |t, s| t.text_wrap_style = s.text_wrap_style),
        "text-indent" => copy_text(style, src, |t, s| t.text_indent = s.text_indent.clone()),
        "tab-size" => copy_text(style, src, |t, s| t.tab_size = s.tab_size),
        "line-break" => copy_text(style, src, |t, s| t.line_break = s.line_break),
        "hyphens" => copy_text(style, src, |t, s| t.hyphens = s.hyphens),
        "hyphenate-character" => copy_text(style, src, |t, s| {
            t.hyphenate_character = s.hyphenate_character.clone()
        }),
        "hyphenate-limit-chars" => copy_text(style, src, |t, s| {
            t.hyphenate_limit_chars = s.hyphenate_limit_chars
        }),
        "hyphenate-limit-lines" => copy_text(style, src, |t, s| {
            t.hyphenate_limit_lines = s.hyphenate_limit_lines
        }),
        "text-autospace" => copy_text(style, src, |t, s| t.text_autospace = s.text_autospace),
        "font-palette" => copy_text(style, src, |t, s| t.font_palette = s.font_palette.clone()),
        "text-spacing-trim" => {
            copy_text(style, src, |t, s| t.text_spacing_trim = s.text_spacing_trim)
        }
        "ruby-position" => copy_text(style, src, |t, s| t.ruby_position = s.ruby_position),
        "ruby-align" => copy_text(style, src, |t, s| t.ruby_align = s.ruby_align),
        "math-style" => copy_text(style, src, |t, s| t.math_style = s.math_style),
        "hanging-punctuation" => copy_text(style, src, |t, s| {
            t.hanging_punctuation = s.hanging_punctuation
        }),
        "word-space-transform" => copy_text(style, src, |t, s| {
            t.word_space_transform = s.word_space_transform
        }),
        "text-fit" => copy_text(style, src, |t, s| t.text_fit = s.text_fit),
        "text-group-align" => copy_text(style, src, |t, s| t.text_group_align = s.text_group_align),
        "unicode-bidi" => copy_text(style, src, |t, s| t.unicode_bidi = s.unicode_bidi),
        "overflow-wrap" => style.overflow_wrap = src.overflow_wrap,
        "word-break" => style.word_break = src.word_break,
        "visibility" => style.visibility = src.visibility,
        "text-shadow" => copy_decoration(style, src, |d, s| d.text_shadow = s.text_shadow.clone()),
        "letter-spacing" => {
            style.letter_spacing = src.letter_spacing;
            copy_text(style, src, |t, s| {
                t.letter_spacing = s.letter_spacing.clone()
            });
        }
        "word-spacing" => {
            style.word_spacing = src.word_spacing;
            copy_text(style, src, |t, s| t.word_spacing = s.word_spacing.clone());
        }
        "width" => style.width = src.width.clone(),
        "height" => style.height = src.height.clone(),
        "margin" => style.margin = src.margin.clone(),
        "margin-top" => style.margin[TOP] = src.margin[TOP].clone(),
        "margin-right" => style.margin[RIGHT] = src.margin[RIGHT].clone(),
        "margin-bottom" => style.margin[BOTTOM] = src.margin[BOTTOM].clone(),
        "margin-left" => style.margin[LEFT] = src.margin[LEFT].clone(),
        "padding" => style.padding = src.padding.clone(),
        "padding-top" => style.padding[TOP] = src.padding[TOP].clone(),
        "padding-right" => style.padding[RIGHT] = src.padding[RIGHT].clone(),
        "padding-bottom" => style.padding[BOTTOM] = src.padding[BOTTOM].clone(),
        "padding-left" => style.padding[LEFT] = src.padding[LEFT].clone(),
        "border-width" => style.border_width = src.border_width,
        "border-top-width" => style.border_width[TOP] = src.border_width[TOP],
        "border-right-width" => style.border_width[RIGHT] = src.border_width[RIGHT],
        "border-bottom-width" => style.border_width[BOTTOM] = src.border_width[BOTTOM],
        "border-left-width" => style.border_width[LEFT] = src.border_width[LEFT],
        "border-style" => style.border_style = src.border_style,
        "border-top-style" => style.border_style[TOP] = src.border_style[TOP],
        "border-right-style" => style.border_style[RIGHT] = src.border_style[RIGHT],
        "border-bottom-style" => style.border_style[BOTTOM] = src.border_style[BOTTOM],
        "border-left-style" => style.border_style[LEFT] = src.border_style[LEFT],
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            let side = match name {
                "border-top-color" => TOP,
                "border-right-color" => RIGHT,
                "border-bottom-color" => BOTTOM,
                _ => LEFT,
            };
            if from_parent {
                style.border_color[side] = parent.border_color[side];
            }
            border_color_set[side] = from_parent;
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let side = match name {
                "border-top" => TOP,
                "border-right" => RIGHT,
                "border-bottom" => BOTTOM,
                _ => LEFT,
            };
            style.border_width[side] = src.border_width[side];
            style.border_style[side] = src.border_style[side];
            if from_parent {
                style.border_color[side] = parent.border_color[side];
            }
            border_color_set[side] = from_parent;
        }
        "overflow" => (style.overflow_x, style.overflow_y) = (src.overflow_x, src.overflow_y),
        "overflow-x" => style.overflow_x = src.overflow_x,
        "overflow-y" => style.overflow_y = src.overflow_y,
        "box-sizing" => style.box_sizing = src.box_sizing,
        "min-width" => style.min_width = src.min_width.clone(),
        "max-width" => style.max_width = src.max_width.clone(),
        "min-height" => style.min_height = src.min_height.clone(),
        "max-height" => style.max_height = src.max_height.clone(),
        "float" => {
            style.float = src.float;
            style.flow_relative.float = src.flow_relative.float;
        }
        "clear" => {
            style.clear = src.clear;
            style.flow_relative.clear = src.flow_relative.clear;
        }
        "resize" => copy_effects(style, src, |e, s| e.resize = s.resize),
        "appearance" => copy_effects(style, src, |e, s| e.appearance = s.appearance),
        "container-type" => style.container_type = src.container_type,
        "container-name" => style.container_names = src.container_names.clone(),
        "container" => {
            style.container_type = src.container_type;
            style.container_names = src.container_names.clone();
        }
        "position" => style.position = src.position,
        "top" | "right" | "bottom" | "left" => {
            let side = edge_index(name);
            style.inset[side] = src.inset[side].clone();
        }
        "inset" => style.inset = src.inset.clone(),
        "z-index" => style.z_index = src.z_index,
        "table-layout" => style.table_layout = src.table_layout,
        "border-collapse" => style.border_collapse = src.border_collapse,
        "border-spacing" => style.border_spacing = src.border_spacing,
        "caption-side" => style.caption_side = src.caption_side,
        "outline" => copy_outline(style, src, |o, s| {
            o.style = s.style;
            o.width = s.width;
            o.color = s.color;
        }),
        "outline-style" => copy_outline(style, src, |o, s| o.style = s.style),
        "outline-width" => copy_outline(style, src, |o, s| o.width = s.width),
        "outline-color" => copy_outline(style, src, |o, s| o.color = s.color),
        "outline-offset" => copy_outline(style, src, |o, s| o.offset = s.offset),
        "cursor" => {
            style.cursor = src.cursor;
            let images = &src.image().cursor_images;
            if !images.is_empty() || !style.image().cursor_images.is_empty() {
                style.image_mut().cursor_images = Arc::clone(images);
            }
        }
        "empty-cells" => style.empty_cells = src.empty_cells,
        "vertical-align" => style.vertical_align = src.vertical_align.clone(),
        "content" => style.content = src.content.clone(),
        "quotes" => style.quotes = src.quotes.clone(),
        "flex-direction" => style.flex_direction = src.flex_direction,
        "flex-wrap" => style.flex_wrap = src.flex_wrap,
        "flex-flow" => {
            style.flex_direction = src.flex_direction;
            style.flex_wrap = src.flex_wrap;
        }
        "order" => style.order = src.order,
        "reading-flow" => copy_grid(style, src, |g, s| g.reading_flow = s.reading_flow),
        "reading-order" => copy_grid(style, src, |g, s| g.reading_order = s.reading_order),
        "contain" => copy_containment(style, src, |c, s| c.contain = s.contain),
        "content-visibility" => copy_containment(style, src, |c, s| {
            c.content_visibility = s.content_visibility
        }),
        "contain-intrinsic-size" => copy_containment(style, src, |c, s| c.intrinsic = s.intrinsic),
        "contain-intrinsic-width" | "contain-intrinsic-inline-size" => {
            copy_containment(style, src, |c, s| c.intrinsic[0] = s.intrinsic[0])
        }
        "contain-intrinsic-height" | "contain-intrinsic-block-size" => {
            copy_containment(style, src, |c, s| c.intrinsic[1] = s.intrinsic[1])
        }
        "flex-grow" => style.flex_grow = src.flex_grow,
        "flex-shrink" => style.flex_shrink = src.flex_shrink,
        "flex-basis" => style.flex_basis = src.flex_basis.clone(),
        "flex" => {
            style.flex_grow = src.flex_grow;
            style.flex_shrink = src.flex_shrink;
            style.flex_basis = src.flex_basis.clone();
        }
        "justify-content" => style.justify_content = src.justify_content,
        "align-content" => style.align_content = src.align_content,
        "place-content" => {
            style.align_content = src.align_content;
            style.justify_content = src.justify_content;
        }
        "align-items" => style.align_items = src.align_items,
        "align-self" => style.align_self = src.align_self,
        // `justify-items`' initial value, `legacy`, computes against the
        // parent's (css-align-3 §6.1).
        "justify-items" | "place-items" => {
            if name == "place-items" {
                style.align_items = src.align_items;
            }
            let value = if from_parent {
                parent.grid().justify_items
            } else {
                JustifyItems::legacy_under(parent.grid().justify_items)
            };
            if style.grid.is_some() || value != JustifyItems::NORMAL {
                style.grid_mut().justify_items = value;
            }
        }
        "justify-self" | "place-self" => {
            if name == "place-self" {
                style.align_self = src.align_self;
            }
            copy_grid(style, src, |g, s| g.justify_self = s.justify_self);
        }
        "grid-template-columns" => copy_grid(style, src, |g, s| {
            g.template_columns = s.template_columns.clone()
        }),
        "grid-template-rows" => {
            copy_grid(style, src, |g, s| g.template_rows = s.template_rows.clone())
        }
        "grid-template-areas" => copy_grid(style, src, |g, s| {
            g.template_areas = s.template_areas.clone()
        }),
        "grid-template" => copy_grid(style, src, |g, s| {
            g.template_columns = s.template_columns.clone();
            g.template_rows = s.template_rows.clone();
            g.template_areas = s.template_areas.clone();
        }),
        "grid-auto-columns" => {
            copy_grid(style, src, |g, s| g.auto_columns = s.auto_columns.clone())
        }
        "grid-auto-rows" => copy_grid(style, src, |g, s| g.auto_rows = s.auto_rows.clone()),
        "grid-auto-flow" => copy_grid(style, src, |g, s| g.auto_flow = s.auto_flow),
        "grid" => copy_grid(style, src, |g, s| {
            g.template_columns = s.template_columns.clone();
            g.template_rows = s.template_rows.clone();
            g.template_areas = s.template_areas.clone();
            g.auto_columns = s.auto_columns.clone();
            g.auto_rows = s.auto_rows.clone();
            g.auto_flow = s.auto_flow;
        }),
        "grid-row-start" => copy_grid(style, src, |g, s| g.row_start = s.row_start.clone()),
        "grid-row-end" => copy_grid(style, src, |g, s| g.row_end = s.row_end.clone()),
        "grid-column-start" => {
            copy_grid(style, src, |g, s| g.column_start = s.column_start.clone())
        }
        "grid-column-end" => copy_grid(style, src, |g, s| g.column_end = s.column_end.clone()),
        "grid-row" => copy_grid(style, src, |g, s| {
            g.row_start = s.row_start.clone();
            g.row_end = s.row_end.clone();
        }),
        "grid-column" => copy_grid(style, src, |g, s| {
            g.column_start = s.column_start.clone();
            g.column_end = s.column_end.clone();
        }),
        "grid-area" => copy_grid(style, src, |g, s| {
            g.row_start = s.row_start.clone();
            g.row_end = s.row_end.clone();
            g.column_start = s.column_start.clone();
            g.column_end = s.column_end.clone();
        }),
        "transform" => copy_effects(style, src, |e, s| e.transform = s.transform.clone()),
        "transform-origin" => copy_effects(style, src, |e, s| {
            e.transform_origin = s.transform_origin.clone()
        }),
        "transform-box" => copy_effects(style, src, |e, s| e.transform_box = s.transform_box),
        "translate" => copy_effects(style, src, |e, s| e.translate = s.translate.clone()),
        "rotate" => copy_effects(style, src, |e, s| e.rotate = s.rotate),
        "scale" => copy_effects(style, src, |e, s| e.scale = s.scale),
        "transform-style" => copy_effects(style, src, |e, s| e.transform_style = s.transform_style),
        "perspective" => copy_effects(style, src, |e, s| e.perspective = s.perspective),
        "perspective-origin" => copy_effects(style, src, |e, s| {
            e.perspective_origin = s.perspective_origin.clone()
        }),
        "backface-visibility" => copy_effects(style, src, |e, s| {
            e.backface_visibility = s.backface_visibility
        }),
        "overlay" => copy_effects(style, src, |e, s| e.overlay = s.overlay),
        "scroll-behavior" => copy_effects(style, src, |e, s| {
            e.scroll_behavior_smooth = s.scroll_behavior_smooth
        }),
        "interactivity" => style.interactivity_inert = src.interactivity_inert,
        "pointer-events" => style.pointer_events = src.pointer_events,
        "interest-delay-start" => {
            copy_effects(style, src, |e, s| e.interest_delay[0] = s.interest_delay[0])
        }
        "interest-delay-end" => {
            copy_effects(style, src, |e, s| e.interest_delay[1] = s.interest_delay[1])
        }
        "interest-delay" => copy_effects(style, src, |e, s| e.interest_delay = s.interest_delay),
        "scroll-snap-type" => {
            copy_effects(style, src, |e, s| e.scroll_snap_type = s.scroll_snap_type)
        }
        "scroll-snap-align" => {
            copy_effects(style, src, |e, s| e.scroll_snap_align = s.scroll_snap_align)
        }
        "scroll-snap-stop" => copy_effects(style, src, |e, s| {
            e.scroll_snap_stop_always = s.scroll_snap_stop_always
        }),
        "scroll-initial-target" => copy_effects(style, src, |e, s| {
            e.scroll_initial_target = s.scroll_initial_target
        }),
        "scroll-margin" => copy_effects(style, src, |e, s| e.scroll_margin = s.scroll_margin),
        "scroll-padding" => copy_effects(style, src, |e, s| {
            e.scroll_padding = s.scroll_padding.clone()
        }),
        "scroll-margin-top"
        | "scroll-margin-right"
        | "scroll-margin-bottom"
        | "scroll-margin-left" => {
            let side = side_of(name);
            copy_effects(style, src, |e, s| {
                e.scroll_margin[side] = s.scroll_margin[side]
            })
        }
        "scroll-padding-top"
        | "scroll-padding-right"
        | "scroll-padding-bottom"
        | "scroll-padding-left" => {
            let side = side_of(name);
            copy_effects(style, src, |e, s| {
                e.scroll_padding[side] = s.scroll_padding[side].clone()
            })
        }
        "opacity" => copy_effects(style, src, |e, s| e.opacity = s.opacity),
        "box-shadow" => copy_effects(style, src, |e, s| e.box_shadow = s.box_shadow.clone()),
        "filter" => copy_effects(style, src, |e, s| e.filter = s.filter.clone()),
        "will-change" => copy_effects(style, src, |e, s| e.will_change = s.will_change),
        "transition-property" => copy_motion(style, src, |m, s| {
            m.transition_property = s.transition_property.clone()
        }),
        "transition-duration" => copy_motion(style, src, |m, s| {
            m.transition_duration = s.transition_duration.clone()
        }),
        "transition-timing-function" => copy_motion(style, src, |m, s| {
            m.transition_timing_function = s.transition_timing_function.clone()
        }),
        "transition-delay" => copy_motion(style, src, |m, s| {
            m.transition_delay = s.transition_delay.clone()
        }),
        "transition-behavior" => copy_motion(style, src, |m, s| {
            m.transition_behavior = s.transition_behavior.clone()
        }),
        "transition" => copy_motion(style, src, |m, s| {
            m.transition_property = s.transition_property.clone();
            m.transition_duration = s.transition_duration.clone();
            m.transition_timing_function = s.transition_timing_function.clone();
            m.transition_delay = s.transition_delay.clone();
            m.transition_behavior = s.transition_behavior.clone();
        }),
        "animation-name" => copy_motion(style, src, |m, s| {
            m.animation_name = s.animation_name.clone()
        }),
        "animation-duration" => copy_motion(style, src, |m, s| {
            m.animation_duration = s.animation_duration.clone()
        }),
        "animation-timing-function" => copy_motion(style, src, |m, s| {
            m.animation_timing_function = s.animation_timing_function.clone()
        }),
        "animation-iteration-count" => copy_motion(style, src, |m, s| {
            m.animation_iteration_count = s.animation_iteration_count.clone()
        }),
        "animation-direction" => copy_motion(style, src, |m, s| {
            m.animation_direction = s.animation_direction.clone()
        }),
        "animation-play-state" => copy_motion(style, src, |m, s| {
            m.animation_play_state = s.animation_play_state.clone()
        }),
        "animation-delay" => copy_motion(style, src, |m, s| {
            m.animation_delay = s.animation_delay.clone()
        }),
        "animation-fill-mode" => copy_motion(style, src, |m, s| {
            m.animation_fill_mode = s.animation_fill_mode.clone()
        }),
        "animation-composition" => copy_motion(style, src, |m, s| {
            m.animation_composition = s.animation_composition.clone()
        }),
        "animation" => copy_motion(style, src, |m, s| {
            m.animation_name = s.animation_name.clone();
            m.animation_duration = s.animation_duration.clone();
            m.animation_timing_function = s.animation_timing_function.clone();
            m.animation_iteration_count = s.animation_iteration_count.clone();
            m.animation_direction = s.animation_direction.clone();
            m.animation_play_state = s.animation_play_state.clone();
            m.animation_delay = s.animation_delay.clone();
            m.animation_fill_mode = s.animation_fill_mode.clone();
            m.animation_composition = s.animation_composition.clone();
            m.animation_timeline = s.animation_timeline.clone();
            m.animation_range_start = s.animation_range_start.clone();
            m.animation_range_end = s.animation_range_end.clone();
        }),
        "break-before" | "page-break-before" => {
            copy_fragmentation(style, src, |f, s| f.break_before = s.break_before)
        }
        "break-after" | "page-break-after" => {
            copy_fragmentation(style, src, |f, s| f.break_after = s.break_after)
        }
        "break-inside" | "page-break-inside" => {
            copy_fragmentation(style, src, |f, s| f.break_inside = s.break_inside)
        }
        "orphans" => copy_fragmentation(style, src, |f, s| f.orphans = s.orphans),
        "counter-reset" => copy_lists(style, src, |l, s| l.counter_reset = s.counter_reset.clone()),
        "counter-increment" => copy_lists(style, src, |l, s| {
            l.counter_increment = s.counter_increment.clone()
        }),
        "counter-set" => copy_lists(style, src, |l, s| l.counter_set = s.counter_set.clone()),
        "list-style-type" => copy_lists(style, src, |l, s| {
            l.list_style_type = s.list_style_type.clone()
        }),
        "list-style-position" => copy_lists(style, src, |l, s| {
            l.list_style_position = s.list_style_position
        }),
        "list-style-image" => copy_lists(style, src, |l, s| {
            l.list_style_image = s.list_style_image.clone()
        }),
        "list-style" => copy_lists(style, src, |l, s| {
            l.list_style_type = s.list_style_type.clone();
            l.list_style_position = s.list_style_position;
            l.list_style_image = s.list_style_image.clone();
        }),
        "aspect-ratio" => copy_image(style, src, |i, s| i.aspect_ratio = s.aspect_ratio),
        "object-fit" => copy_image(style, src, |i, s| i.object_fit = s.object_fit),
        "object-position" => copy_image(style, src, |i, s| {
            i.object_position = s.object_position.clone()
        }),
        "image-orientation" => {
            copy_image(style, src, |i, s| i.image_orientation = s.image_orientation)
        }
        "image-rendering" => copy_image(style, src, |i, s| i.image_rendering = s.image_rendering),
        "text-decoration-line" => copy_decoration(style, src, |d, s| d.line = s.line),
        "text-decoration-style" => copy_decoration(style, src, |d, s| d.style = s.style),
        "text-decoration-color" => copy_decoration(style, src, |d, s| d.color = s.color),
        "text-decoration-thickness" => {
            copy_decoration(style, src, |d, s| d.thickness = s.thickness.clone())
        }
        "text-decoration" => copy_decoration(style, src, |d, s| {
            d.line = s.line;
            d.style = s.style;
            d.color = s.color;
            d.thickness = s.thickness.clone();
        }),
        "text-underline-offset" => copy_decoration(style, src, |d, s| {
            d.underline_offset = s.underline_offset.clone()
        }),
        "text-underline-position" => copy_decoration(style, src, |d, s| {
            d.underline_position = s.underline_position
        }),
        "widows" => copy_fragmentation(style, src, |f, s| f.widows = s.widows),
        "print-color-adjust" => copy_fragmentation(style, src, |f, s| {
            f.print_color_adjust = s.print_color_adjust
        }),
        "row-gap" => style.row_gap = src.row_gap.clone(),
        "column-gap" => style.column_gap = src.column_gap.clone(),
        "gap" => {
            style.row_gap = src.row_gap.clone();
            style.column_gap = src.column_gap.clone();
        }
        "border-color" => {
            // The parent's border color is a real color; the initial
            // value is `currentcolor`, resolved once the cascade is done.
            if from_parent {
                style.border_color = parent.border_color;
            }
            *border_color_set = [from_parent; 4];
        }
        "border" => {
            style.border_width = src.border_width;
            style.border_style = src.border_style;
            style.border_image = src.border_image.clone();
            if from_parent {
                style.border_color = parent.border_color;
            }
            *border_color_set = [from_parent; 4];
        }
        // SVG's (M32); else a property this engine does not know:
        // ignored, as ever.
        _ => return svg::copy(style, src, name),
    }
    true
}

/// Applies a flow-relative declaration as the physical ones `mapped`
/// names (D74): a longhand as its physical longhand; a shorthand's one or
/// two values to its start and end, or its whole value to both sides, all
/// or, when one is invalid, none.
fn apply_logical(
    style: &mut ComputedStyle,
    mapped: logical::Mapped,
    value: &[Token],
    bases: &LengthBases<'_>,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) -> bool {
    dom::hook!("style::apply_logical");
    let groups;
    let (names, values): ([String; 2], [&[Token]; 2]) = match mapped {
        logical::Mapped::One(physical) => {
            return apply_declaration(style, &physical, value, bases, parent, border_color_set);
        }
        logical::Mapped::Both(sides) => (sides, [value, value]),
        logical::Mapped::Pair(sides) => {
            groups = split_components(value);
            match groups.as_slice() {
                [one] => (sides, [one.as_slice(), one.as_slice()]),
                [start, end] => (sides, [start.as_slice(), end.as_slice()]),
                _ => return false,
            }
        }
    };
    let mut trial = style.clone();
    let mut set = *border_color_set;
    let applied = names
        .iter()
        .zip(values)
        .all(|(name, value)| apply_declaration(&mut trial, name, value, bases, parent, &mut set));
    if applied {
        *style = trial;
        *border_color_set = set;
    }
    applied
}

/// Like [`set_edge`], for a `margin-*` longhand (which additionally
/// accepts `auto`).
fn set_margin_edge(
    edges: &mut MarginEdges,
    which: usize,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> bool {
    set(
        &mut edges[which],
        parse_length_percentage_or_auto(tokens, bases),
    )
}

/// Expands a 1–4 value list into `[top, right, bottom, left]` per CSS2
/// §8.3's box-shorthand expansion rule (shared by `margin`, `padding`,
/// and `border-width` — only the per-value parser differs).
fn expand_1_to_4<T: Clone>(values: &[T]) -> Option<[T; 4]> {
    let [a, b, c, d] = match values {
        [a] => [a, a, a, a],
        [a, b] => [a, b, a, b],
        [a, b, c] => [a, b, c, b],
        [a, b, c, d] => [a, b, c, d],
        _ => return None,
    };
    Some([a.clone(), b.clone(), c.clone(), d.clone()])
}

/// Expands the `padding` box shorthand — like [`parse_box_shorthand`],
/// but each value may also be a percentage.
fn parse_padding_shorthand(tokens: &[Token], bases: &LengthBases<'_>) -> Option<PaddingEdges> {
    let groups = split_components(tokens);
    let values: Option<Vec<LengthPercentage>> = groups
        .iter()
        .map(|g| parse_non_negative_length_percentage(g, bases))
        .collect();
    expand_1_to_4(&values?)
}

/// A `scroll-padding` side: `auto`, or a length or percentage not
/// below zero.
fn parse_scroll_padding(tokens: &[Token], bases: &LengthBases<'_>) -> Option<LengthPercentageAuto> {
    if is_auto(tokens) {
        return Some(LengthPercentageAuto::Auto);
    }
    parse_non_negative_length_percentage(tokens, bases).map(|v| match v {
        LengthPercentage::Px(px) => LengthPercentageAuto::Px(px),
        LengthPercentage::Percent(p) => LengthPercentageAuto::Percent(p),
        LengthPercentage::Calc(c) => LengthPercentageAuto::Calc(c),
    })
}

/// The side a `-top`, `-right`, `-bottom` or `-left` property names.
pub(crate) fn side_of(name: &str) -> usize {
    if name.ends_with("-top") {
        TOP
    } else if name.ends_with("-right") {
        RIGHT
    } else if name.ends_with("-bottom") {
        BOTTOM
    } else {
        LEFT
    }
}

/// Expands the `margin` box shorthand — like [`parse_box_shorthand`],
/// but each value may also be a percentage or `auto`.
fn parse_margin_shorthand(tokens: &[Token], bases: &LengthBases<'_>) -> Option<MarginEdges> {
    let groups = split_components(tokens);
    let values: Option<Vec<LengthPercentageAuto>> = groups
        .iter()
        .map(|g| parse_length_percentage_or_auto(g, bases))
        .collect();
    expand_1_to_4(&values?)
}

/// The `border` shorthand and the `border-{side}` ones: any order, any
/// subset, of a width, a style keyword, and a color, written to every
/// side in `sides`. Using a shorthand always resets the parts it
/// doesn't mention to their initial value (real shorthand semantics):
/// `medium` for the width, `none` for the style, and an omitted color
/// re-enables the `currentcolor` default applied once the whole cascade
/// for this element is done (see [`compute_style`]).
fn apply_border_shorthand(
    style: &mut ComputedStyle,
    sides: &[usize],
    tokens: &[Token],
    bases: &LengthBases<'_>,
    border_color_set: &mut [bool; 4],
) -> bool {
    let mut width = None;
    let mut border_style = None;
    let mut color = None;
    let groups = split_components(tokens);
    if groups.is_empty() {
        return false; // `border: ;` has no component: an invalid declaration, ignored
    }
    for g in groups {
        if width.is_none()
            && let Some(w) = parse_border_width(&g, bases)
        {
            width = Some(w);
        } else if border_style.is_none()
            && let Some(s) = parse_border_style(&g)
        {
            border_style = Some(s);
        } else if color.is_none()
            && let Some(c) = parse_border_color(&g, bases.colors())
        {
            color = Some(c);
        } else {
            return false; // an unrecognized (or repeated) part: drop the whole shorthand
        }
    }
    for &side in sides {
        style.border_width[side] = width.unwrap_or(3.0);
        style.border_style[side] = border_style.unwrap_or(BorderStyle::None);
        write_border_color(style, side, color.flatten(), border_color_set);
    }
    true
}

/// The component values of `tokens`, each copied: what the shorthands
/// and the lists of keywords take one value from each of. White space
/// between two is optional, as CSS Syntax 3 has it, and white space
/// inside a function or a block splits nothing: `calc(1px + 2px) 3px` is
/// two values (M21), and so are `1px/**/2px` and `var(--a)var(--b)` once
/// substituted, which the reference browser takes as two.
fn split_components(tokens: &[Token]) -> Vec<Vec<Token>> {
    components(tokens)
        .into_iter()
        .map(<[Token]>::to_vec)
        .collect()
}

/// The component values of `tokens` (CSS Syntax 3 §5.4.7), the white
/// space between them left out: each a token, or a function or block
/// with everything up to its closer, or to the end where it has none.
pub(crate) fn components(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], Token::Whitespace) {
            i += 1;
            continue;
        }
        let start = i;
        let mut nesting = Nesting::default();
        loop {
            nesting.step(&tokens[i]);
            i += 1;
            if nesting.is_top() || i == tokens.len() {
                break;
            }
        }
        out.push(&tokens[start..i]);
    }
    out
}

/// A function's name and its arguments, `component` being one: the
/// component values between its commas, each one value, there being at
/// least one; `None` when it is no function, is not closed, or an
/// argument is empty or more than one value.
fn function_arguments(component: &[Token]) -> Option<(&str, Vec<&[Token]>)> {
    let (Token::Function(name), [inside @ .., Token::CloseParen]) =
        (component.first()?, &component[1..])
    else {
        return None;
    };
    let mut arguments = Vec::new();
    let mut nesting = Nesting::default();
    let mut start = 0;
    for (i, token) in inside.iter().enumerate() {
        if matches!(token, Token::Comma) && nesting.is_top() {
            arguments.push(&inside[start..i]);
            start = i + 1;
        } else {
            nesting.step(token);
        }
    }
    arguments.push(&inside[start..]);
    arguments
        .iter()
        .all(|a| components(a).len() == 1)
        .then_some((name.as_str(), arguments))
}

/// The blocks open at a token of a value, by CSS Syntax 3's component
/// values: a function or `(` until its `)`, a `[` until its `]`, a `{`
/// until its `}`; a closer with no block of its kind open closes none.
#[derive(Debug, Default)]
pub(crate) struct Nesting {
    open: Vec<u8>,
}

impl Nesting {
    /// Takes `token` into account.
    pub(crate) fn step(&mut self, token: &Token) {
        match token {
            Token::Function(_) | Token::OpenParen => self.open.push(b')'),
            Token::OpenSquare => self.open.push(b']'),
            Token::OpenCurly => self.open.push(b'}'),
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                let closer = match token {
                    Token::CloseParen => b')',
                    Token::CloseSquare => b']',
                    _ => b'}',
                };
                if self.open.last() == Some(&closer) {
                    self.open.pop();
                }
            }
            _ => {}
        }
    }

    /// Whether no block is open.
    pub(crate) fn is_top(&self) -> bool {
        self.open.is_empty()
    }
}

/// The largest length, in CSS pixels, a declaration can set in either
/// direction. Layout arithmetic in `f32` stays finite and exact enough
/// well past it; Blink's layout unit stops at the same figure.
pub const MAX_LENGTH_PX: f32 = 33_554_431.0;

/// A length in CSS pixels (css-values-4 §6): the absolute units, the
/// font-relative `em`, `rem`, `ex` and `ch`, the viewport-percentage
/// `vw`, `vh`, `vmin` and `vmax`, or the unitless `0` (the one number
/// CSS allows without a unit for length properties). Anything else (a
/// nonzero unitless number, a percentage, an unknown unit) is not a
/// length. A non-finite value is rejected and a finite one clamped to
/// [`MAX_LENGTH_PX`] in magnitude — see the module doc for what an
/// infinity does to layout otherwise.
fn parse_length_px(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    length(tokens, bases, Range::All)
}

/// The values a place takes: any, or none below zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Range {
    All,
    NonNegative,
}

impl Range {
    /// `v`, an infinity or a value past [`MAX_LENGTH_PX`] made the
    /// largest the place takes and one below its least made that
    /// (css-values-4 §10.9.2, §10.12): how a math function's value is
    /// fitted to its place, where a value written out of range is
    /// invalid.
    pub(crate) fn clamp(self, v: f64) -> f32 {
        let low = match self {
            Range::All => -f64::from(MAX_LENGTH_PX),
            Range::NonNegative => 0.0,
        };
        v.clamp(low, f64::from(MAX_LENGTH_PX)) as f32
    }

    /// Whether `v`, written as it is, is in the range.
    fn holds(self, v: f32) -> bool {
        self == Range::All || v >= 0.0
    }
}

/// `value` of `unit` in CSS pixels, by the element's font and the
/// viewport for the relative units.
fn unit_px(value: f64, unit: LengthUnit, bases: &LengthBases<'_>) -> f64 {
    if let Some(px) = unit.absolute() {
        return value * px;
    }
    let r = bases.resolution;
    let (vw, vh) = (r.context.viewport_width, r.context.viewport_height);
    let viewport = |size: f32| {
        r.viewport_used.set(true);
        value * f64::from(size) / 100.0
    };
    let em = f64::from(bases.em);
    // The root's font, the initial one on the root itself.
    let root = r.root_font.unwrap_or(crate::computed::initial_font());
    // A unit of a font's figures or a line: what a web font's arrival
    // and a change of the root's font compute again.
    let used = || {
        if let Some(used) = r.fonts_used {
            used.set(true);
        }
    };
    let figures = |font: &FontProps, size: f32| match r.context.fonts {
        Some(fonts) => fonts.units(font, size),
        None => FontUnits::fallback(size),
    };
    if !matches!(
        unit,
        LengthUnit::Em
            | LengthUnit::Rem
            | LengthUnit::Vw
            | LengthUnit::Vh
            | LengthUnit::Vmin
            | LengthUnit::Vmax
            | LengthUnit::Cqw
            | LengthUnit::Cqh
            | LengthUnit::Cqi
            | LengthUnit::Cqb
            | LengthUnit::Cqmin
            | LengthUnit::Cqmax
    ) {
        used();
    }
    // The query containers' sizes, the small viewport's where there is
    // none (D72).
    let container = |pick: fn(f32, f32) -> f32| {
        let (width, height) = r.container_units();
        value * f64::from(pick(width, height)) / 100.0
    };
    let line = |(font, size): (&FontProps, f32)| match font.line_height {
        crate::computed::LineHeight::Normal => figures(font, size).normal_line_height,
        crate::computed::LineHeight::Number(n) => n * size,
        crate::computed::LineHeight::Px(px) => px,
    };
    let of = |figure: f32| value * f64::from(figure);
    match unit {
        LengthUnit::Em => value * em,
        LengthUnit::Rem => value * f64::from(r.rem),
        LengthUnit::Ex => of(figures(bases.font, bases.em).x_height),
        LengthUnit::Ch => of(figures(bases.font, bases.em).zero_advance),
        LengthUnit::Cap => of(figures(bases.font, bases.em).cap_height),
        LengthUnit::Ic => of(figures(bases.font, bases.em).ic_advance),
        LengthUnit::Lh => of(line(bases.line)),
        LengthUnit::Rex => of(figures(root, r.root_size).x_height),
        LengthUnit::Rch => of(figures(root, r.root_size).zero_advance),
        LengthUnit::Rcap => of(figures(root, r.root_size).cap_height),
        LengthUnit::Ric => of(figures(root, r.root_size).ic_advance),
        LengthUnit::Rlh => of(line((root, r.root_size))),
        LengthUnit::Vw => viewport(vw),
        LengthUnit::Vh => viewport(vh),
        LengthUnit::Vmin => viewport(vw.min(vh)),
        LengthUnit::Vmax => viewport(vw.max(vh)),
        LengthUnit::Cqw | LengthUnit::Cqi => container(|w, _| w),
        LengthUnit::Cqh | LengthUnit::Cqb => container(|_, h| h),
        LengthUnit::Cqmin => container(f32::min),
        LengthUnit::Cqmax => container(f32::max),
        _ => value,
    }
}

impl calc::Lengths for LengthBases<'_> {
    fn px(&self, value: f64, unit: LengthUnit) -> f64 {
        unit_px(value, unit, self)
    }
}

/// A math function (M21.2) in `tokens` for a place that takes `want`,
/// computed for the element: its percentages of `basis` when that is
/// known. `None` when `tokens` are not one math function, or it is not
/// one the place takes.
fn math(
    tokens: &[Token],
    want: Want,
    bases: &LengthBases<'_>,
    basis: Option<f64>,
) -> Option<calc::Computed> {
    if !calc::is_math_function(tokens) {
        return None;
    }
    calc::parse(tokens, want).map(|parsed| parsed.compute(bases, basis))
}

/// A `<length>` in `range`, in CSS pixels: a dimension of one of the
/// units css-values-4 §6 defines that this engine takes — the absolute
/// ones, `em`, `rem`, `ex`, `ch`, `vw`, `vh`, `vmin` and `vmax` — the
/// unitless `0`, or a math function. A written length out of `range`,
/// or not finite, is no length; a math function's is fitted to it.
fn length(tokens: &[Token], bases: &LengthBases<'_>, range: Range) -> Option<f32> {
    if let Some(calc::Computed::Value(v)) = math(tokens, Want::Length, bases, None) {
        return Some(range.clamp(v));
    }
    let px = match single_token(tokens)? {
        // A value past what an `f32` holds is no length, whatever its
        // unit would make of it.
        Token::Dimension { value, unit } if (*value as f32).is_finite() => {
            unit_px(*value, LengthUnit::of(unit)?, bases) as f32
        }
        Token::Number { value, .. } if *value == 0.0 => 0.0,
        _ => return None,
    };
    (px.is_finite() && range.holds(px)).then(|| px.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX))
}

/// [`parse_length_px`] for the properties that take no negative value
/// (`font-size`, `padding`, `border-width`): a negative one is an
/// invalid declaration, ignored.
fn parse_non_negative_length_px(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    length(tokens, bases, Range::NonNegative)
}

/// A `<length-percentage>` in `range`: a length, a percentage kept as
/// written, or a math function, which is a length, a percentage, or an
/// expression with a percentage in it kept for layout (M21.2).
fn length_percentage(
    tokens: &[Token],
    bases: &LengthBases<'_>,
    range: Range,
) -> Option<LengthPercentage> {
    length_percentage_of(tokens, bases, range, false)
}

/// [`length_percentage`] of a basis that may be negative, a
/// `<position>`'s where `signed` (gap batch E).
fn length_percentage_of(
    tokens: &[Token],
    bases: &LengthBases<'_>,
    range: Range,
    signed: bool,
) -> Option<LengthPercentage> {
    let computed = calc::is_math_function(tokens)
        .then(|| calc::parse(tokens, Want::LengthPercentage))
        .flatten()
        .map(|p| if signed { p.of_a_signed_basis() } else { p })
        .map(|p| p.compute(bases, None));
    match computed {
        Some(calc::Computed::Value(v)) => return Some(LengthPercentage::Px(range.clamp(v))),
        // A percentage below zero is of a basis never below it, so it
        // is fitted to a non-negative range as a length is; an infinite
        // one is the largest a percentage holds (D78).
        Some(calc::Computed::Percent(p)) => {
            let p = p.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
            return Some(LengthPercentage::Percent(match range {
                Range::All => p,
                Range::NonNegative => p.max(0.0),
            }));
        }
        Some(calc::Computed::Expression(e)) => {
            return Some(LengthPercentage::Calc(Arc::new(Calc::new(
                e,
                range == Range::NonNegative,
            ))));
        }
        None => {}
    }
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        let p = *value as f32;
        return (p.is_finite() && range.holds(p)).then_some(LengthPercentage::Percent(p));
    }
    length(tokens, bases, range).map(LengthPercentage::Px)
}

/// A non-negative length or percentage, kept as written — `padding`.
fn parse_non_negative_length_percentage(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<LengthPercentage> {
    length_percentage(tokens, bases, Range::NonNegative)
}

/// `auto`, a length, or a percentage — the margins (M10.1) and the
/// insets (M11). A percentage is kept as written for layout to resolve
/// against the containing block.
fn parse_length_percentage_or_auto(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<LengthPercentageAuto> {
    if is_auto(tokens) {
        return Some(LengthPercentageAuto::Auto);
    }
    length_percentage(tokens, bases, Range::All).map(|v| match v {
        LengthPercentage::Px(px) => LengthPercentageAuto::Px(px),
        LengthPercentage::Percent(p) => LengthPercentageAuto::Percent(p),
        LengthPercentage::Calc(c) => LengthPercentageAuto::Calc(c),
    })
}

fn is_auto(tokens: &[Token]) -> bool {
    single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("auto"))
}

/// `display` as its text is (css-display-3 section 2), the value it
/// computes to before blockification and whether its box is a flow root:
/// for a script's specified value, written as the computed one is
/// ([`crate::serialize`]); `None` for a value the engine does not take.
pub fn parse_display_value(tokens: &[Token]) -> Option<(Display, bool)> {
    parse_display(tokens)
}

fn parse_display(tokens: &[Token]) -> Option<(Display, bool)> {
    if let Some(multi) = parse_multi_keyword_display(tokens) {
        return Some(multi);
    }
    let display = match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "none" => Some(Display::None),
        "table" => Some(Display::Table),
        "inline-table" => Some(Display::InlineTable),
        "table-row-group" => Some(Display::TableRowGroup),
        "table-header-group" => Some(Display::TableHeaderGroup),
        "table-footer-group" => Some(Display::TableFooterGroup),
        "table-row" => Some(Display::TableRow),
        "table-column-group" => Some(Display::TableColumnGroup),
        "table-column" => Some(Display::TableColumn),
        "table-cell" => Some(Display::TableCell),
        "table-caption" => Some(Display::TableCaption),
        // The `-webkit-` forms are the Compatibility Standard's (§3.4.5).
        "flex" | "-webkit-flex" => Some(Display::Flex),
        "inline-flex" | "-webkit-inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        // css-display-3 section 2.5 and its internal ruby values (D77).
        "contents" => Some(Display::Contents),
        "ruby-base" => Some(Display::RubyBase),
        "ruby-text" => Some(Display::RubyText),
        "ruby-base-container" => Some(Display::RubyBaseContainer),
        "ruby-text-container" => Some(Display::RubyTextContainer),
        // `run-in` is not taken: the reference browser does not take it
        // (D77), alone or with an inside value.
        _ => None,
    }?;
    Some((display, false))
}

/// `display`'s multi-keyword values (css-display-3 section 2, gap batch
/// E's `flow-root` and D77's): an outside value (`block` or `inline`)
/// and an inside one (`flow`, `flow-root`, `table`, `flex`, `grid`,
/// `ruby`, `math`), each at most once in either order, the outside one
/// `block` where it is left out but for `ruby` and `math`, which are
/// inline; or a list item, `list-item` with an outside value and `flow`
/// or `flow-root`. A lone keyword that is a legacy or a box value is
/// [`parse_display`]'s. The value and whether it is a flow root.
fn parse_multi_keyword_display(tokens: &[Token]) -> Option<(Display, bool)> {
    let mut outside = None::<bool>;
    let mut inside = None::<&'static str>;
    let mut list_item = false;
    let mut count = 0;
    for token in tokens {
        let Token::Ident(word) = token else {
            if matches!(token, Token::Whitespace) {
                continue;
            }
            return None;
        };
        count += 1;
        match word.to_ascii_lowercase().as_str() {
            "block" | "inline" if outside.is_none() => {
                outside = Some(word.eq_ignore_ascii_case("inline"));
            }
            "list-item" if !list_item => list_item = true,
            other if inside.is_none() => {
                inside = Some(match other {
                    "flow" => "flow",
                    "flow-root" => "flow-root",
                    "table" => "table",
                    "flex" => "flex",
                    "grid" => "grid",
                    "ruby" => "ruby",
                    "math" => "math",
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    // A single keyword of these is a legacy value or `flow-root`'s,
    // `flow`'s, `ruby`'s and `math`'s own: the table there names them.
    if count == 1 && !list_item && !matches!(inside, Some("flow" | "flow-root" | "ruby" | "math")) {
        return None;
    }
    let inline = outside.unwrap_or(matches!(inside, Some("ruby" | "math")) && !list_item);
    if list_item {
        return match inside {
            None | Some("flow") => Some((Display::list_item(inline), false)),
            Some("flow-root") => Some((Display::list_item(inline), true)),
            _ => None,
        };
    }
    Some(match (inside?, inline) {
        ("flow", false) => (Display::Block, false),
        ("flow", true) => (Display::Inline, false),
        ("flow-root", false) => (Display::Block, true),
        ("flow-root", true) => (Display::InlineBlock, false),
        ("table", false) => (Display::Table, false),
        ("table", true) => (Display::InlineTable, false),
        ("flex", false) => (Display::Flex, false),
        ("flex", true) => (Display::InlineFlex, false),
        ("grid", false) => (Display::Grid, false),
        ("grid", true) => (Display::InlineGrid, false),
        ("ruby", false) => (Display::BlockRuby, false),
        ("ruby", true) => (Display::Ruby, false),
        ("math", false) => (Display::BlockMath, false),
        _ => (Display::Math, false),
    })
}

/// `table-layout` (M12).
fn parse_table_layout(tokens: &[Token]) -> Option<TableLayout> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "auto" => Some(TableLayout::Auto),
        "fixed" => Some(TableLayout::Fixed),
        _ => None,
    }
}

/// `border-collapse` (M12).
fn parse_border_collapse(tokens: &[Token]) -> Option<BorderCollapse> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "separate" => Some(BorderCollapse::Separate),
        "collapse" => Some(BorderCollapse::Collapse),
        _ => None,
    }
}

/// `border-spacing` (M12): one or two non-negative lengths, horizontal
/// then vertical, one value standing for both (CSS2 §17.6.1). No
/// percentage.
fn parse_border_spacing(tokens: &[Token], bases: &LengthBases<'_>) -> Option<BorderSpacing> {
    let groups = split_components(tokens);
    let (horizontal, vertical) = match groups.as_slice() {
        [one] => {
            let v = parse_non_negative_length_px(one, bases)?;
            (v, v)
        }
        [first, second] => (
            parse_non_negative_length_px(first, bases)?,
            parse_non_negative_length_px(second, bases)?,
        ),
        _ => return None,
    };
    Some(BorderSpacing {
        horizontal,
        vertical,
    })
}

/// `outline-style` (CSS UI 4 section 5.3).
fn parse_outline_style(tokens: &[Token]) -> Option<OutlineStyle> {
    OutlineStyle::from_keyword(single_ident(tokens)?)
}

/// `outline-color`: a colour, or `auto` or `currentcolor`, `None` (CSS
/// UI 4 section 5.4); the reference browser's focus ring colour is
/// `-webkit-focus-ring-color`.
fn parse_outline_color(tokens: &[Token], colors: Colors) -> Option<Option<Color>> {
    match single_ident(tokens) {
        Some(ident)
            if ident.eq_ignore_ascii_case("auto") || ident.eq_ignore_ascii_case("currentcolor") =>
        {
            Some(None)
        }
        _ => color_value(tokens, colors).map(Some),
    }
}

/// The `outline` shorthand (CSS UI 4 section 5.1): a colour, a style and
/// a width, each at most once, in any order; those left out are reset.
fn apply_outline_shorthand(
    style: &mut ComputedStyle,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> bool {
    let groups = split_components(tokens);
    if groups.is_empty() {
        return false;
    }
    let mut width = None;
    let mut outline_style = None;
    let mut color = None;
    for g in groups {
        if width.is_none()
            && let Some(w) = parse_border_width(&g, bases)
        {
            width = Some(w);
        } else if outline_style.is_none()
            && let Some(s) = parse_outline_style(&g)
        {
            outline_style = Some(s);
        } else if color.is_none()
            && let Some(c) = parse_outline_color(&g, bases.colors())
        {
            color = Some(c);
        } else {
            return false;
        }
    }
    let initial = Outline::default();
    let outline = style.outline_mut();
    outline.width = width.unwrap_or(initial.width);
    outline.style = outline_style.unwrap_or(initial.style);
    outline.color = color.flatten();
    true
}

/// `cursor` (CSS UI 4 section 4.1): `[ <url> [<x> <y>]? , ]*` and a
/// keyword; the images are not shown, so the keyword is the value.
fn parse_cursor(tokens: &[Token]) -> Option<(Cursor, Vec<crate::computed::CursorImage>)> {
    let mut parts = tokens.split(|token| matches!(token, Token::Comma));
    let last = parts.next_back()?;
    let mut images = Vec::new();
    for image in parts {
        let words: Vec<&Token> = trim(image)
            .iter()
            .filter(|t| !matches!(t, Token::Whitespace))
            .collect();
        let url = match words.first() {
            Some(Token::Url { value, bad: false }) => value.clone(),
            Some(Token::Function(name)) if name.eq_ignore_ascii_case("url") => match words.get(1) {
                Some(Token::String { value, bad: false }) => value.clone(),
                _ => return None,
            },
            _ => return None,
        };
        let rest = match words.first() {
            Some(Token::Function(_)) => &words[3.min(words.len())..],
            _ => &words[1..],
        };
        let number = |t: &Token| match t {
            Token::Number { value, .. } if value.is_finite() => Some(*value as f32),
            _ => None,
        };
        let hotspot = match rest {
            [] => None,
            [x, y] => Some([number(x)?, number(y)?]),
            _ => return None,
        };
        if images.len() < MAX_CURSOR_IMAGES {
            images.push((Arc::from(url.as_str()), hotspot));
        }
    }
    Some((Cursor::from_keyword(single_ident(last)?)?, images))
}

/// The most images a `cursor` keeps.
const MAX_CURSOR_IMAGES: usize = 16;

/// `caption-side` (M12): `top` or `bottom`; css-logical's `inline-start`
/// and `inline-end` are not accepted.
fn parse_caption_side(tokens: &[Token]) -> Option<CaptionSide> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "top" => Some(CaptionSide::Top),
        "bottom" => Some(CaptionSide::Bottom),
        _ => None,
    }
}

/// `empty-cells` (M12).
fn parse_empty_cells(tokens: &[Token]) -> Option<EmptyCells> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "show" => Some(EmptyCells::Show),
        "hide" => Some(EmptyCells::Hide),
        _ => None,
    }
}

/// `vertical-align` (M12): the eight keywords of CSS2 §10.8.1, a
/// length (negative allowed) or a percentage, the last kept as written
/// since it is of the line height.
fn parse_vertical_align(tokens: &[Token], bases: &LengthBases<'_>) -> Option<VerticalAlign> {
    if let Some(ident) = single_ident(tokens) {
        return match ident.to_ascii_lowercase().as_str() {
            "baseline" => Some(VerticalAlign::Baseline),
            "sub" => Some(VerticalAlign::Sub),
            "super" => Some(VerticalAlign::Super),
            "top" => Some(VerticalAlign::Top),
            "text-top" => Some(VerticalAlign::TextTop),
            "middle" => Some(VerticalAlign::Middle),
            "bottom" => Some(VerticalAlign::Bottom),
            "text-bottom" => Some(VerticalAlign::TextBottom),
            "-webkit-baseline-middle" => Some(VerticalAlign::BaselineMiddle),
            _ => None,
        };
    }
    length_percentage(tokens, bases, Range::All).map(|v| match v {
        LengthPercentage::Px(px) => VerticalAlign::Length(px),
        LengthPercentage::Percent(p) => VerticalAlign::Percent(p),
        LengthPercentage::Calc(c) => VerticalAlign::Calc(c),
    })
}

/// The edge array index of `top`, `right`, `bottom` or `left`.
fn edge_index(name: &str) -> usize {
    match name {
        "top" => TOP,
        "right" => RIGHT,
        "bottom" => BOTTOM,
        _ => LEFT,
    }
}

/// `direction` (Writing Modes 4 section 2.1, D74): `ltr` or `rtl`.
fn parse_direction(tokens: &[Token]) -> Option<Direction> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "ltr" => Some(Direction::Ltr),
        "rtl" => Some(Direction::Rtl),
        _ => None,
    }
}

/// A `direction` declaration's value, a CSS-wide keyword's among them,
/// for an element whose parent's style is `parent`: `None` when it is
/// not one the property takes.
fn parse_direction_value(tokens: &[Token], parent: &ComputedStyle) -> Option<Direction> {
    match css_wide_keyword(tokens) {
        Some(CssWide::Initial) => Some(Direction::Ltr),
        Some(CssWide::Inherit | CssWide::Unset) => Some(parent.direction),
        None => parse_direction(tokens),
    }
}

/// The physical side a flow-relative value of `float` or `clear` is
/// in a box whose containing block's direction is `direction`.
fn inline_side(word: &str, direction: Direction) -> Option<(bool, InlineSide)> {
    let side = match word {
        "inline-start" => InlineSide::Start,
        "inline-end" => InlineSide::End,
        _ => return None,
    };
    let left = (side == InlineSide::Start) == (direction == Direction::Ltr);
    Some((left, side))
}

/// `float` (M11.1), with the flow-relative values (CSS Logical 1 section
/// 3, D74) made the physical side `direction`, the parent's, makes them.
fn parse_float(tokens: &[Token], direction: Direction) -> Option<(Float, Option<InlineSide>)> {
    let word = single_ident(tokens)?.to_ascii_lowercase();
    if let Some((left, side)) = inline_side(&word, direction) {
        return Some((if left { Float::Left } else { Float::Right }, Some(side)));
    }
    let float = match word.as_str() {
        "none" => Float::None,
        "left" => Float::Left,
        "right" => Float::Right,
        _ => return None,
    };
    Some((float, None))
}

/// `clear` (M11.1), its flow-relative values as `float`'s.
fn parse_clear(tokens: &[Token], direction: Direction) -> Option<(Clear, Option<InlineSide>)> {
    let word = single_ident(tokens)?.to_ascii_lowercase();
    if let Some((left, side)) = inline_side(&word, direction) {
        return Some((if left { Clear::Left } else { Clear::Right }, Some(side)));
    }
    let clear = match word.as_str() {
        "none" => Clear::None,
        "left" => Clear::Left,
        "right" => Clear::Right,
        "both" => Clear::Both,
        _ => return None,
    };
    Some((clear, None))
}

/// `resize` (CSS UI 4 section 5.1), with the flow-relative `block` and
/// `inline` (CSS Logical 1 section 3).
fn parse_resize(tokens: &[Token]) -> Option<Resize> {
    Some(match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" => Resize::None,
        "both" => Resize::Both,
        "horizontal" => Resize::Horizontal,
        "vertical" => Resize::Vertical,
        "block" => Resize::Block,
        "inline" => Resize::Inline,
        _ => return None,
    })
}

/// `position` (M11.3 to M11.5).
fn parse_position(tokens: &[Token]) -> Option<Position> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "static" => Some(Position::Static),
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        "sticky" => Some(Position::Sticky),
        _ => None,
    }
}

/// `z-index` (M11.6): `auto`, or an integer; `Some(None)` is `auto`.
fn parse_z_index(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Option<i32>> {
    if is_auto(tokens) {
        return Some(None);
    }
    integer(tokens, bases).map(Some)
}

/// A lone `<integer>`, one beyond what an `i32` holds clamped to it
/// (css-values-4 §5: a value outside the range an implementation
/// supports is clamped, not dropped); or a math function, whose number
/// is rounded to the nearest integer, a half towards positive infinity
/// (§10.9).
fn integer(tokens: &[Token], bases: &LengthBases<'_>) -> Option<i32> {
    let value = match math(tokens, Want::Number, bases, None) {
        Some(calc::Computed::Value(v)) => (v + 0.5).floor(),
        _ => match single_token(tokens)? {
            Token::Number {
                value,
                is_integer: true,
            } => *value,
            _ => return None,
        },
    };
    Some(value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
}

/// `box-sizing` (M10.2).
fn parse_box_sizing(tokens: &[Token]) -> Option<BoxSizing> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

/// A size keyword of css-sizing-3 §3.2 (M17), shared by [`Size`] and
/// [`MaxSize`].
fn size_keyword(name: &str) -> Option<Size> {
    match name.to_ascii_lowercase().as_str() {
        "min-content" => Some(Size::MinContent),
        "max-content" => Some(Size::MaxContent),
        "fit-content" => Some(Size::FitContent),
        _ => None,
    }
}

/// `width`, `height`, `min-width` and `min-height` (M10.3, M17): `auto`,
/// a non-negative length or percentage, or `min-content`,
/// `max-content` or `fit-content` (css-sizing-3 §3.2).
fn parse_size(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Size> {
    if let Some(name) = single_ident(tokens) {
        if name.eq_ignore_ascii_case("auto") {
            return Some(Size::Auto);
        }
        return size_keyword(name);
    }
    parse_non_negative_length_percentage(tokens, bases).map(|v| match v {
        LengthPercentage::Px(px) => Size::Px(px),
        LengthPercentage::Percent(p) => Size::Percent(p),
        LengthPercentage::Calc(c) => Size::Calc(c),
    })
}

/// `max-width` and `max-height` (M10.3, M17): `none`, a non-negative
/// length or percentage, or an intrinsic keyword.
fn parse_max_size(tokens: &[Token], bases: &LengthBases<'_>) -> Option<MaxSize> {
    if let Some(name) = single_ident(tokens) {
        if name.eq_ignore_ascii_case("none") {
            return Some(MaxSize::None);
        }
        return size_keyword(name).map(|keyword| match keyword {
            Size::MinContent => MaxSize::MinContent,
            Size::MaxContent => MaxSize::MaxContent,
            _ => MaxSize::FitContent,
        });
    }
    parse_non_negative_length_percentage(tokens, bases).map(|v| match v {
        LengthPercentage::Px(px) => MaxSize::Px(px),
        LengthPercentage::Percent(p) => MaxSize::Percent(p),
        LengthPercentage::Calc(c) => MaxSize::Calc(c),
    })
}

/// `flex-direction` (css-flexbox-1 §5.1, M17).
fn parse_flex_direction(tokens: &[Token]) -> Option<FlexDirection> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

/// `flex-wrap` (css-flexbox-1 §5.2, M17).
fn parse_flex_wrap(tokens: &[Token]) -> Option<FlexWrap> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

/// `flex-flow` (css-flexbox-1 §5.3, M17): a direction, a wrap, or both
/// in either order, each at most once; the one left out takes its
/// initial value.
fn parse_flex_flow(tokens: &[Token]) -> Option<(FlexDirection, FlexWrap)> {
    let groups = split_components(tokens);
    if groups.is_empty() {
        return None;
    }
    // Each part at most once: a third is always a repeat.
    let (mut direction, mut wrap) = (None, None);
    for g in &groups {
        if direction.is_none()
            && let Some(d) = parse_flex_direction(g)
        {
            direction = Some(d);
        } else if wrap.is_none()
            && let Some(w) = parse_flex_wrap(g)
        {
            wrap = Some(w);
        } else {
            return None;
        }
    }
    Some((
        direction.unwrap_or(FlexDirection::Row),
        wrap.unwrap_or(FlexWrap::NoWrap),
    ))
}

/// `order` (css-flexbox-1 §5.4, M17): an integer.
fn parse_order(tokens: &[Token], bases: &LengthBases<'_>) -> Option<i32> {
    integer(tokens, bases)
}

/// A flex factor (`flex-grow`, `flex-shrink`; css-flexbox-1 §7.2): a
/// number not below zero, as large as `f32` holds.
fn flex_factor(value: f64) -> Option<f32> {
    (value.is_finite() && value >= 0.0).then(|| value.min(f32::MAX as f64) as f32)
}

/// A flex factor in `tokens`: `None` when they are not a number,
/// `Some(None)` when they are one no factor can be. A math function's
/// number is fitted to a factor's range (M21.2).
fn factor(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Option<f32>> {
    if let Some(Token::Number { value, .. }) = single_token(tokens) {
        return Some(flex_factor(*value));
    }
    match math(tokens, Want::Number, bases, None)? {
        calc::Computed::Value(v) => Some(Some(v.clamp(0.0, f64::from(f32::MAX)) as f32)),
        _ => None,
    }
}

/// `flex-grow` and `flex-shrink` alone.
fn parse_flex_factor(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    factor(tokens, bases)?
}

/// `flex-basis` (css-flexbox-1 §7.2.3, M17): `content`, or what `width`
/// takes.
fn parse_flex_basis(tokens: &[Token], bases: &LengthBases<'_>) -> Option<FlexBasis> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("content")) {
        return Some(FlexBasis::Content);
    }
    parse_size(tokens, bases).map(FlexBasis::Size)
}

/// The `flex` shorthand (css-flexbox-1 §7.1, M17): `none`, `auto`, or
/// a grow factor followed at once by an optional shrink factor, and a
/// basis before or after them. A unitless zero is a factor unless two
/// came before it. The grow and shrink factors left out are `1`, and a
/// basis left out is `0%`: the draft says `0`, but the
/// web-platform-tests and the browsers have `0%`.
fn parse_flex(tokens: &[Token], bases: &LengthBases<'_>) -> Option<(f32, f32, FlexBasis)> {
    if let Some(keyword) = single_ident(tokens) {
        match keyword.to_ascii_lowercase().as_str() {
            "none" => return Some((0.0, 0.0, FlexBasis::Size(Size::Auto))),
            "auto" => return Some((1.0, 1.0, FlexBasis::Size(Size::Auto))),
            _ => {}
        }
    }
    let groups = split_components(tokens);
    if groups.is_empty() || groups.len() > 3 {
        return None;
    }
    let (mut grow, mut shrink, mut basis) = (None, None, None);
    let mut after_grow = false;
    for g in &groups {
        if let Some(value) = factor(g, bases)
            && !(grow.is_some() && shrink.is_some())
        {
            if grow.is_none() {
                grow = Some(value?);
                after_grow = true;
                continue;
            }
            if after_grow {
                shrink = Some(value?);
                after_grow = false;
                continue;
            }
            // A factor that is neither the grow factor nor the shrink
            // factor right after it.
            return None;
        }
        after_grow = false;
        if basis.is_some() {
            return None;
        }
        basis = Some(parse_flex_basis(g, bases)?);
    }
    Some((
        grow.unwrap_or(1.0),
        shrink.unwrap_or(1.0),
        basis.unwrap_or(FlexBasis::Size(Size::Percent(0.0))),
    ))
}

/// A positional alignment keyword (css-align-3 §4.1).
fn align_position(name: &str) -> Option<AlignPosition> {
    match name.to_ascii_lowercase().as_str() {
        "center" => Some(AlignPosition::Center),
        "start" => Some(AlignPosition::Start),
        "end" => Some(AlignPosition::End),
        "self-start" => Some(AlignPosition::SelfStart),
        "self-end" => Some(AlignPosition::SelfEnd),
        "flex-start" => Some(AlignPosition::FlexStart),
        "flex-end" => Some(AlignPosition::FlexEnd),
        "left" => Some(AlignPosition::Left),
        "right" => Some(AlignPosition::Right),
        _ => None,
    }
}

/// The identifiers of `groups`, one per group, or `None` when a group
/// is anything else.
fn idents(groups: &[Vec<Token>]) -> Option<Vec<String>> {
    groups
        .iter()
        .map(|g| single_ident(g).map(str::to_ascii_lowercase))
        .collect()
}

/// `baseline`, `first baseline` or `last baseline` (css-align-3 §4.2).
fn baseline_position(words: &[String]) -> Option<BaselinePosition> {
    match words {
        [b] if b == "baseline" => Some(BaselinePosition::First),
        [f, b] if f == "first" && b == "baseline" => Some(BaselinePosition::First),
        [l, b] if l == "last" && b == "baseline" => Some(BaselinePosition::Last),
        _ => None,
    }
}

/// `[ safe | unsafe ]? <position>`, the position one of `allowed`
/// (css-align-3 §4.4).
fn positional(
    words: &[String],
    allowed: &[AlignPosition],
) -> Option<(AlignPosition, OverflowAlign)> {
    let (overflow, position) = match words {
        [p] => (OverflowAlign::Default, p),
        [o, p] if o == "safe" => (OverflowAlign::Safe, p),
        [o, p] if o == "unsafe" => (OverflowAlign::Unsafe, p),
        _ => return None,
    };
    let position = align_position(position).filter(|p| allowed.contains(p))?;
    Some((position, overflow))
}

/// `justify-content` (`justify`) or `align-content` (css-align-3 §5.1,
/// M17), from its words: `justify-content` takes `left` and `right`
/// and no baseline.
fn content_alignment(words: &[String], justify: bool) -> Option<ContentAlignment> {
    use AlignPosition::*;
    if let [w] = words {
        match w.as_str() {
            "normal" => return Some(ContentAlignment::Normal),
            "space-between" => return Some(ContentAlignment::SpaceBetween),
            "space-around" => return Some(ContentAlignment::SpaceAround),
            "space-evenly" => return Some(ContentAlignment::SpaceEvenly),
            "stretch" => return Some(ContentAlignment::Stretch),
            _ => {}
        }
    }
    if !justify && let Some(b) = baseline_position(words) {
        return Some(ContentAlignment::Baseline(b));
    }
    let allowed: &[AlignPosition] = if justify {
        &[Center, Start, End, FlexStart, FlexEnd, Left, Right]
    } else {
        &[Center, Start, End, FlexStart, FlexEnd]
    };
    positional(words, allowed).map(|(p, o)| ContentAlignment::Position(p, o))
}

/// `align-items`, or `align-self` with `auto` (css-align-3 §6, M17),
/// from its words.
fn self_alignment(words: &[String], allow_auto: bool) -> Option<SelfAlignment> {
    use AlignPosition::*;
    if let [w] = words {
        match w.as_str() {
            "auto" if allow_auto => return Some(SelfAlignment::Auto),
            "normal" => return Some(SelfAlignment::Normal),
            "stretch" => return Some(SelfAlignment::Stretch),
            _ => {}
        }
    }
    if let Some(b) = baseline_position(words) {
        return Some(SelfAlignment::Baseline(b));
    }
    positional(
        words,
        &[Center, Start, End, SelfStart, SelfEnd, FlexStart, FlexEnd],
    )
    .map(|(p, o)| SelfAlignment::Position(p, o))
}

/// A two-part alignment shorthand's words split at the first place
/// where both halves parse: `first(words[..k])` and `second(words[k..])`,
/// the second `None` when it is left out.
fn split_pair<A, B>(
    words: &[String],
    first: impl Fn(&[String]) -> Option<A>,
    second: impl Fn(&[String]) -> Option<B>,
) -> Option<(A, Option<B>)> {
    (1..=words.len()).find_map(|k| {
        let a = first(&words[..k])?;
        if k == words.len() {
            return Some((a, None));
        }
        Some((a, Some(second(&words[k..])?)))
    })
}

/// `place-content` (css-align-3 §5.2, M17): `align-content`, then
/// `justify-content`, which when left out is the first value, or
/// `start` when that is a baseline.
fn parse_place_content(tokens: &[Token]) -> Option<(ContentAlignment, ContentAlignment)> {
    let words = idents(&split_components(tokens))?;
    let (align, justify) = split_pair(
        &words,
        |w| content_alignment(w, false),
        |w| content_alignment(w, true),
    )?;
    let justify = justify.unwrap_or(match align {
        ContentAlignment::Baseline(_) => {
            ContentAlignment::Position(AlignPosition::Start, OverflowAlign::Default)
        }
        other => other,
    });
    Some((align, justify))
}

/// `place-items` (css-align-3 §6.3, M17; the justify half since M18):
/// the `align-items` value, then the `justify-items` value, the first
/// value again when it is left out, a bare `legacy` computing against
/// `parent`, the parent's `justify-items`.
fn parse_place_items(
    tokens: &[Token],
    parent: JustifyItems,
) -> Option<(SelfAlignment, JustifyItems)> {
    let words = idents(&split_components(tokens))?;
    let (align, justify) = split_pair(
        &words,
        |w| self_alignment(w, false),
        |w| grid::justify_items(w, parent),
    )?;
    let justify = match justify {
        Some(justify) => justify,
        None => grid::justify_items(&words, parent)?,
    };
    Some((align, justify))
}

/// `place-self` (css-align-3 §6.2.1, M17; the justify half since M18):
/// the `align-self` value, then the `justify-self` value, the first
/// value again when it is left out.
fn parse_place_self(tokens: &[Token]) -> Option<(SelfAlignment, SelfAlignment)> {
    let words = idents(&split_components(tokens))?;
    let (align, justify) = split_pair(
        &words,
        |w| self_alignment(w, true),
        |w| grid::justify_self(w, true),
    )?;
    let justify = match justify {
        Some(justify) => justify,
        None => grid::justify_self(&words, true)?,
    };
    Some((align, justify))
}

/// `row-gap` and `column-gap` (CSS Gaps 1 §2, M17): `normal`, or a
/// non-negative length or percentage. The outer `Option` is "parsed at
/// all"; the inner is `None` for `normal`.
fn parse_gap(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Option<LengthPercentage>> {
    if is_normal(tokens) {
        return Some(None);
    }
    parse_non_negative_length_percentage(tokens, bases).map(Some)
}

/// The `gap` shorthand: the row gap, then the column gap, which is the
/// row gap when left out.
fn parse_gap_shorthand(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<(Option<LengthPercentage>, Option<LengthPercentage>)> {
    match split_components(tokens).as_slice() {
        [row] => {
            let row = parse_gap(row, bases)?;
            Some((row.clone(), row))
        }
        [row, column] => Some((parse_gap(row, bases)?, parse_gap(column, bases)?)),
        _ => None,
    }
}

/// The name a property `name` (lowercase) is known by: an alias's
/// property, every other name as it is (M25, a script's `style`).
pub fn property_name(name: &str) -> &str {
    canonical_name(name)
}

/// The name a property is known by here: a `-webkit-` alias the
/// Compatibility Standard requires (§3.4.1), or a legacy gap name
/// css-align-3 §8.3 requires (M18), as the property itself, every other
/// name as it is.
pub(crate) fn canonical_name(name: &str) -> &str {
    match name {
        "grid-row-gap" => "row-gap",
        "grid-column-gap" => "column-gap",
        "grid-gap" => "gap",
        "-webkit-align-content" => "align-content",
        "-webkit-align-items" => "align-items",
        "-webkit-align-self" => "align-self",
        "-webkit-flex" => "flex",
        "-webkit-flex-basis" => "flex-basis",
        "-webkit-flex-direction" => "flex-direction",
        "-webkit-flex-flow" => "flex-flow",
        "-webkit-flex-grow" => "flex-grow",
        "-webkit-flex-shrink" => "flex-shrink",
        "-webkit-flex-wrap" => "flex-wrap",
        "-webkit-justify-content" => "justify-content",
        "-webkit-order" => "order",
        "-webkit-box-sizing" => "box-sizing",
        "-webkit-appearance" => "appearance",
        "word-wrap" => "overflow-wrap",
        "-webkit-transform" => "transform",
        "-webkit-transform-origin" => "transform-origin",
        "-webkit-transform-style" => "transform-style",
        "-webkit-perspective" => "perspective",
        "-webkit-perspective-origin" => "perspective-origin",
        "-webkit-backface-visibility" => "backface-visibility",
        // Color Adjust 1's legacy name.
        "-webkit-print-color-adjust" => "print-color-adjust",
        // The Compatibility Standard's for transitions and animations
        // (M22).
        "-webkit-transition" => "transition",
        "-webkit-transition-delay" => "transition-delay",
        "-webkit-transition-duration" => "transition-duration",
        "-webkit-transition-property" => "transition-property",
        "-webkit-transition-timing-function" => "transition-timing-function",
        "-webkit-animation" => "animation",
        "-webkit-animation-name" => "animation-name",
        "-webkit-animation-duration" => "animation-duration",
        "-webkit-animation-timing-function" => "animation-timing-function",
        "-webkit-animation-iteration-count" => "animation-iteration-count",
        "-webkit-animation-direction" => "animation-direction",
        "-webkit-animation-play-state" => "animation-play-state",
        "-webkit-animation-delay" => "animation-delay",
        "-webkit-animation-fill-mode" => "animation-fill-mode",
        // The rest of the aliases the reference browser keeps, each the
        // name of the property it writes (a probe of Chrome 156).
        "-webkit-filter" => "filter",
        "-webkit-box-shadow" => "box-shadow",
        "-webkit-border-radius" => "border-radius",
        "-webkit-border-top-left-radius" => "border-top-left-radius",
        "-webkit-border-top-right-radius" => "border-top-right-radius",
        "-webkit-border-bottom-left-radius" => "border-bottom-left-radius",
        "-webkit-border-bottom-right-radius" => "border-bottom-right-radius",
        "-webkit-background-size" => "background-size",
        "-webkit-background-origin" => "background-origin",
        "-webkit-background-clip" => "background-clip",
        "-webkit-clip-path" => "clip-path",
        "-webkit-font-feature-settings" => "font-feature-settings",
        "-webkit-column-count" => "column-count",
        "-webkit-column-gap" => "column-gap",
        "-webkit-column-width" => "column-width",
        "-webkit-columns" => "columns",
        "-webkit-column-rule" => "column-rule",
        "-webkit-column-rule-color" => "column-rule-color",
        "-webkit-column-rule-style" => "column-rule-style",
        "-webkit-column-rule-width" => "column-rule-width",
        "-webkit-column-span" => "column-span",
        "-webkit-shape-outside" => "shape-outside",
        "-webkit-shape-margin" => "shape-margin",
        "-webkit-shape-image-threshold" => "shape-image-threshold",
        "-webkit-text-emphasis" => "text-emphasis",
        "-webkit-text-emphasis-color" => "text-emphasis-color",
        "-webkit-text-emphasis-style" => "text-emphasis-style",
        "-webkit-text-emphasis-position" => "text-emphasis-position",
        "-webkit-opacity" => "opacity",
        "-webkit-mask" => "mask",
        "-webkit-mask-image" => "mask-image",
        "-webkit-mask-size" => "mask-size",
        "-webkit-mask-position" => "mask-position",
        "-webkit-mask-repeat" => "mask-repeat",
        "-webkit-mask-clip" => "mask-clip",
        "-webkit-mask-origin" => "mask-origin",
        "-webkit-mask-composite" => "mask-composite",
        "-webkit-user-select" => "user-select",
        "-webkit-text-size-adjust" => "text-size-adjust",
        "-webkit-margin-start" => "margin-inline-start",
        "-webkit-margin-end" => "margin-inline-end",
        "-webkit-padding-start" => "padding-inline-start",
        "-webkit-padding-end" => "padding-inline-end",
        "-webkit-logical-width" => "inline-size",
        "-webkit-logical-height" => "block-size",
        "-webkit-min-logical-width" => "min-inline-size",
        "-webkit-max-logical-width" => "max-inline-size",
        "-webkit-border-start" => "border-inline-start",
        "-webkit-border-end" => "border-inline-end",
        other => other,
    }
}

/// A keyframe's `animation-timing-function` (Animations 1 §3.2, M22),
/// read apart from any element: a math function's lengths against the
/// initial font size and the default viewport; a list's first item.
pub(crate) fn keyframe_easing(tokens: &[Token]) -> Option<crate::easing::Easing> {
    easings_apart(tokens)?.into_iter().next()
}

/// A list of easing functions read apart from any element, as
/// [`keyframe_easing`] reads one; `None` when it does not parse.
pub(crate) fn easings_apart(tokens: &[Token]) -> Option<Vec<crate::easing::Easing>> {
    apart(|bases| motion::parse_easings(tokens, bases))
}

/// A `<number>` read apart from any element, a math function's among
/// them, as [`keyframe_easing`] reads an easing; `None` when it is none.
pub(crate) fn number_apart(tokens: &[Token]) -> Option<f64> {
    match tokens {
        [Token::Number { value, .. }] => Some(*value),
        _ if calc::is_math_function(tokens) => {
            apart(
                |bases| match calc::parse(tokens, calc::Want::Number)?.compute(bases, None) {
                    calc::Computed::Value(n) => Some(n),
                    _ => None,
                },
            )
        }
        _ => None,
    }
}

/// The initial style with the declarations of `text` applied, read
/// apart from any element as [`keyframe_easing`] reads an easing: what
/// a composite operation adds a value onto as its zero (D75).
pub(crate) fn style_apart(text: &str) -> ComputedStyle {
    let parent = ComputedStyle::default();
    let mut style = ComputedStyle::default();
    let declarations = css::parse_style_attribute(text).unwrap_or_default();
    apart(|bases| {
        for declaration in &declarations {
            apply_declaration(
                &mut style,
                canonical_name(&declaration.name),
                &declaration.value,
                bases,
                &parent,
                &mut [true; 4],
            );
        }
    });
    style
}

/// The longhands `declarations` set, each with its computed value as
/// `getComputedStyle` writes it, the declarations applied onto `base`
/// apart from any element, and the custom properties' values as their
/// tokens: what `getKeyframes` gives of a CSS animation's keyframe or of
/// a computed value (Animations 2 section 4.1). Each property once, at
/// its first place, its last declaration's value.
pub fn declared_values(
    base: &ComputedStyle,
    declarations: &[css::Declaration],
) -> Vec<(String, String)> {
    let mut style = base.clone();
    let mut names: Vec<String> = Vec::new();
    let mut custom: Vec<(String, String)> = Vec::new();
    apart(|bases| {
        for declaration in declarations {
            let name = canonical_name(&declaration.name);
            if custom::is_custom_property_name(name) {
                custom.retain(|(n, _)| n != name);
                custom.push((
                    name.to_owned(),
                    crate::serialize::tokens(&declaration.value),
                ));
                continue;
            }
            // Its `var()`s of the element's custom properties.
            let specified = substituted(
                &declaration.value,
                None,
                name,
                base.custom.as_ref(),
                bases.resolution,
            );
            let value: &[Token] = match &specified {
                Specified::Written(value) => value,
                Specified::Substituted(value) => value,
                Specified::Invalid | Specified::Ignored => continue,
            };
            // The font size has a pass of its own in the cascade.
            if name == "font-size" {
                let Some(size) =
                    declared_font_size(Specified::Written(value), false, base, bases.resolution)
                else {
                    continue;
                };
                style.font_size_px = size;
            } else if !apply_declaration(&mut style, name, value, bases, base, &mut [true; 4]) {
                continue;
            }
            for longhand in crate::animate::longhands_of(name) {
                if longhand.is_animatable() && !names.iter().any(|n| n == longhand.name()) {
                    names.push(longhand.name().to_owned());
                }
            }
        }
    });
    let mut out: Vec<(String, String)> = names
        .into_iter()
        .filter_map(|name| {
            let value = crate::serialize::computed_value(&style, &name)?;
            Some((name, value))
        })
        .collect();
    out.extend(custom);
    out
}

/// `f` of the bases a value is read with apart from any element: a math
/// function's lengths against the initial font size and the default
/// viewport.
fn apart<T>(f: impl FnOnce(&LengthBases<'_>) -> T) -> T {
    let context = CascadeContext::default();
    let viewport_used = Cell::new(false);
    let substitutions = Substitutions::new(0);
    let initial = ComputedStyle::default().font_size_px;
    let resolution = Resolution {
        rem: initial,
        root: false,
        page_scheme: None,
        context: &context,
        viewport_used: &viewport_used,
        fonts_used: None,
        root_font: None,
        root_size: initial,
        scheme_read: None,
        attributes: &[],
        substitutions: &substitutions,
        containers: None,
        units_from: None,
        containers_used: &viewport_used,
    };
    let bases = LengthBases {
        em: initial,
        font: crate::computed::initial_font(),
        line: (crate::computed::initial_font(), initial),
        resolution: &resolution,
        color: Color::BLACK,
        dark: false,
    };
    f(&bases)
}

/// `overflow` (M9.3): the four keywords, one value for both axes. The
/// two-value form of css-overflow-3 (`overflow: hidden visible`) and
/// `clip` are not accepted yet.
/// The `overflow` shorthand's one or two keywords: across, then down.
fn parse_overflow_pair(tokens: &[Token]) -> Option<(Overflow, Overflow)> {
    let words: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let one = |t: &Token| parse_overflow(std::slice::from_ref(t));
    match words[..] {
        [x] => one(x).map(|x| (x, x)),
        [x, y] => Some((one(x)?, one(y)?)),
        _ => None,
    }
}

fn parse_overflow(tokens: &[Token]) -> Option<Overflow> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        "hidden" => Some(Overflow::Hidden),
        "scroll" => Some(Overflow::Scroll),
        "auto" => Some(Overflow::Auto),
        "clip" => Some(Overflow::Clip),
        _ => None,
    }
}

/// A `text-align` value (css-text-3 §7.1, M21.7): one of
/// `text-align-all`'s, `match-parent`, or `justify-all`, which only the
/// shorthand takes.
enum TextAlignValue {
    Align(TextAlign),
    MatchParent,
    JustifyAll,
    /// `-webkit-left`, `-webkit-right`, `-webkit-center` (D60).
    Legacy(TextAlign, crate::computed::LegacyAlign),
}

fn parse_text_align(tokens: &[Token]) -> Option<TextAlignValue> {
    Some(match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "start" => TextAlignValue::Align(TextAlign::Start),
        "end" => TextAlignValue::Align(TextAlign::End),
        "left" => TextAlignValue::Align(TextAlign::Left),
        "right" => TextAlignValue::Align(TextAlign::Right),
        "center" => TextAlignValue::Align(TextAlign::Center),
        "justify" => TextAlignValue::Align(TextAlign::Justify),
        "match-parent" => TextAlignValue::MatchParent,
        "-webkit-left" => {
            TextAlignValue::Legacy(TextAlign::Left, crate::computed::LegacyAlign::Left)
        }
        "-webkit-right" => {
            TextAlignValue::Legacy(TextAlign::Right, crate::computed::LegacyAlign::Right)
        }
        "-webkit-center" => {
            TextAlignValue::Legacy(TextAlign::Center, crate::computed::LegacyAlign::Center)
        }
        "justify-all" => TextAlignValue::JustifyAll,
        _ => return None,
    })
}

/// `text-align-last: match-parent`: the parent's value, `start` and
/// `end` made `left` or `right` by its direction; `auto` stays.
fn matched_last(parent: &ComputedStyle) -> TextAlignLast {
    match parent.text_align_last {
        TextAlignLast::Auto => TextAlignLast::Auto,
        TextAlignLast::Align(align) => TextAlignLast::Align(align.matched(parent.direction)),
    }
}

/// `text-justify` (css-text-3 §7.4, M21.7); `distribute` is
/// `inter-character`.
fn parse_text_justify(tokens: &[Token]) -> Option<TextJustify> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "auto" => Some(TextJustify::Auto),
        "none" => Some(TextJustify::None),
        "inter-word" => Some(TextJustify::InterWord),
        "inter-character" | "distribute" => Some(TextJustify::InterCharacter),
        _ => None,
    }
}

/// `white-space` (CSS Text 4 section 3): one of its six keywords, or a
/// `white-space-collapse` value and a `text-wrap-mode` one, in either
/// order, each at most once, the one left out initial.
fn parse_white_space(tokens: &[Token]) -> Option<WhiteSpace> {
    let words = split_components(tokens);
    if let [one] = words.as_slice() {
        let keyword = match single_ident(one)?.to_ascii_lowercase().as_str() {
            "normal" => Some(WhiteSpace::NORMAL),
            "pre" => Some(WhiteSpace::PRE),
            "pre-wrap" => Some(WhiteSpace::PRE_WRAP),
            "pre-line" => Some(WhiteSpace::PRE_LINE),
            "nowrap" => Some(WhiteSpace::NOWRAP),
            "break-spaces" => Some(WhiteSpace::BREAK_SPACES),
            _ => None,
        };
        if keyword.is_some() {
            return keyword;
        }
    }
    let (mut collapse, mut wrap) = (None, None);
    for word in &words {
        if let Some(c) = parse_white_space_collapse(word) {
            if collapse.replace(c).is_some() {
                return None;
            }
        } else if wrap.replace(parse_text_wrap_mode(word)?).is_some() {
            return None;
        }
    }
    (!words.is_empty()).then(|| WhiteSpace {
        collapse: collapse.unwrap_or(WhiteSpaceCollapse::Collapse),
        wrap: wrap.unwrap_or(true),
    })
}

/// `white-space-collapse`.
fn parse_white_space_collapse(tokens: &[Token]) -> Option<WhiteSpaceCollapse> {
    Some(match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "collapse" => WhiteSpaceCollapse::Collapse,
        "preserve" => WhiteSpaceCollapse::Preserve,
        "preserve-breaks" => WhiteSpaceCollapse::PreserveBreaks,
        "break-spaces" => WhiteSpaceCollapse::BreakSpaces,
        "preserve-spaces" => WhiteSpaceCollapse::PreserveSpaces,
        _ => return None,
    })
}

/// `text-wrap-mode`: whether lines wrap.
fn parse_text_wrap_mode(tokens: &[Token]) -> Option<bool> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "wrap" => Some(true),
        "nowrap" => Some(false),
        _ => None,
    }
}

/// `overflow-wrap`.
fn parse_overflow_wrap(tokens: &[Token]) -> Option<crate::computed::OverflowWrap> {
    use crate::computed::OverflowWrap as O;
    Some(match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "normal" => O::Normal,
        "anywhere" => O::Anywhere,
        "break-word" => O::BreakWord,
        _ => return None,
    })
}

/// `word-break`.
fn parse_word_break(tokens: &[Token]) -> Option<crate::computed::WordBreak> {
    use crate::computed::WordBreak as W;
    Some(match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "normal" => W::Normal,
        "break-all" => W::BreakAll,
        "keep-all" => W::KeepAll,
        "break-word" => W::BreakWord,
        "auto-phrase" => W::AutoPhrase,
        "manual" => W::Manual,
        _ => return None,
    })
}

/// `visibility`.
fn parse_visibility(tokens: &[Token]) -> Option<crate::computed::Visibility> {
    use crate::computed::Visibility as V;
    Some(match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "visible" => V::Visible,
        "hidden" => V::Hidden,
        "collapse" => V::Collapse,
        _ => return None,
    })
}

fn is_normal(tokens: &[Token]) -> bool {
    single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("normal"))
}

fn parse_border_style(tokens: &[Token]) -> Option<BorderStyle> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" => Some(BorderStyle::None),
        "hidden" => Some(BorderStyle::Hidden),
        "solid" => Some(BorderStyle::Solid),
        "double" => Some(BorderStyle::Double),
        "dashed" => Some(BorderStyle::Dashed),
        "dotted" => Some(BorderStyle::Dotted),
        "groove" => Some(BorderStyle::Groove),
        "ridge" => Some(BorderStyle::Ridge),
        "inset" => Some(BorderStyle::Inset),
        "outset" => Some(BorderStyle::Outset),
        _ => None,
    }
}

/// A `font-family` list (CSS Fonts 4 section 2.1), written as the
/// reference browser writes it (`crate::font_family`): what the text
/// crate chooses faces by.
fn parse_font_family(tokens: &[Token]) -> Option<String> {
    crate::font_family::parse_list(tokens).map(|list| crate::font_family::write_list(&list))
}

/// `color-scheme`'s value (Color Adjust 1 section 2.1): `normal`, or
/// one or more scheme words and `only` at most once, kept as written
/// with `only` last; `None` where it is not one. A CSS-wide keyword is
/// the parent's or `normal`.
fn color_scheme_of(tokens: &[Token], parent: &ComputedStyle) -> Option<Option<Arc<String>>> {
    match css_wide_keyword(tokens) {
        Some(CssWide::Initial) => return Some(None),
        Some(_) => return Some(parent.color_scheme.clone()),
        None => {}
    }
    let mut words: Vec<&str> = Vec::new();
    let mut only = false;
    let idents: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let last = idents.len().saturating_sub(1);
    for (i, token) in idents.into_iter().enumerate() {
        let Token::Ident(word) = token else {
            return None;
        };
        // `only` first or last, once.
        if word.eq_ignore_ascii_case("only") && !only && (i == 0 || i == last) {
            only = true;
            continue;
        }
        let lower = word.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "normal"
                | "only"
                | "default"
                | "inherit"
                | "initial"
                | "unset"
                | "revert"
                | "revert-layer"
        ) || words.len() >= MAX_SCHEME_WORDS
        {
            return single_ident(tokens)
                .filter(|w| w.eq_ignore_ascii_case("normal"))
                .map(|_| None);
        }
        words.push(word);
    }
    if words.is_empty() {
        return None;
    }
    let mut text = words.join(" ");
    if only {
        text.push_str(" only");
    }
    Some(Some(Arc::new(text)))
}

/// The page's supported colour schemes (HTML 4.2.5.4): the first
/// `meta` named `color-scheme` in the document's head whose content is
/// a `color-scheme` value other than `normal`.
fn page_scheme(document: &Document) -> Option<Arc<String>> {
    let root = document.document_element()?;
    let head = document
        .element_children(root)
        .find(|&n| document.tag(n) == Some("head"))?;
    document.subtree(head).find_map(|n| {
        if document.tag(n) != Some("meta")
            || !document
                .attr(n, "name")
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("color-scheme"))
        {
            return None;
        }
        let tokens = css::tokenize(document.attr(n, "content")?);
        css_wide_keyword(&tokens)
            .is_none()
            .then(|| color_scheme_of(&tokens, &ComputedStyle::default()))
            .flatten()
            .flatten()
    })
}

/// The corner a `border-*-radius` longhand names, in [`Radii`]'s order.
fn corner_of(name: &str) -> usize {
    match name {
        "border-top-left-radius" => 0,
        "border-top-right-radius" => 1,
        "border-bottom-right-radius" => 2,
        _ => 3,
    }
}

/// A `border-*-radius` longhand: one or two non-negative lengths or
/// percentages, horizontal then vertical.
fn parse_corner_radius(tokens: &[Token], bases: &LengthBases<'_>) -> Option<[LengthPercentage; 2]> {
    let groups = split_components(tokens);
    let values: Vec<LengthPercentage> = groups
        .iter()
        .map(|g| parse_non_negative_length_percentage(g, bases))
        .collect::<Option<_>>()?;
    match values.as_slice() {
        [one] => Some([one.clone(), one.clone()]),
        [h, v] => Some([h.clone(), v.clone()]),
        _ => None,
    }
}

/// `border-radius`: one to four horizontal radii, then after a `/` one
/// to four vertical ones, each list expanded as `margin`'s is from the
/// top-left corner clockwise; no `/`, the vertical ones the same.
fn parse_border_radius(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Radii> {
    let mut halves = tokens.split(|t| matches!(t, Token::Delim('/')));
    let horizontal = halves.next()?;
    let vertical = halves.next();
    if halves.next().is_some() {
        return None;
    }
    let list = |part: &[Token]| -> Option<[LengthPercentage; 4]> {
        let values: Vec<LengthPercentage> = split_components(part)
            .iter()
            .map(|g| parse_non_negative_length_percentage(g, bases))
            .collect::<Option<_>>()?;
        expand_1_to_4(&values)
    };
    let h = list(horizontal)?;
    let v = match vertical {
        Some(part) => list(part)?,
        None => h.clone(),
    };
    Some(Radii(std::array::from_fn(|i| [h[i].clone(), v[i].clone()])))
}

/// `box-shadow` (Backgrounds 3 section 7.1, gap batch E, D52): `none`,
/// or shadows separated by commas, each two to four lengths, the offsets,
/// the blur radius, never negative, and the spread, with a colour and
/// `inset` before or after them, each at most once. At most
/// [`MAX_SHADOWS`]; a longer list is no value. `text-shadow` (D56) when
/// `text`: no spread and no `inset`.
fn parse_shadows(
    tokens: &[Token],
    bases: &LengthBases<'_>,
    text: bool,
) -> Option<Vec<crate::computed::Shadow>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(Vec::new());
    }
    let mut shadows = Vec::new();
    for item in split_top_level_commas(tokens) {
        let parts = split_components(item);
        let mut lengths: Vec<f32> = Vec::new();
        let mut calculated: Vec<bool> = Vec::new();
        let mut color = None;
        let mut inset = false;
        // Where the lengths stand: one run, nothing between them.
        let mut run_ended = false;
        for part in &parts {
            if !text && single_ident(part).is_some_and(|w| w.eq_ignore_ascii_case("inset")) {
                if inset {
                    return None;
                }
                inset = true;
                run_ended |= !lengths.is_empty();
            } else if let Some(px) = length(part, bases, Range::All) {
                if run_ended {
                    return None;
                }
                lengths.push(px);
                calculated.push(crate::calc::is_math_function(part));
            } else if color.is_none()
                && let Some(c) = color_value(part, bases.colors())
            {
                color = Some(c);
                run_ended |= !lengths.is_empty();
            } else {
                return None;
            }
        }
        let [x, y, blur, spread] = match lengths.as_slice() {
            [x, y] => [*x, *y, 0.0, 0.0],
            [x, y, b] => [*x, *y, *b, 0.0],
            [x, y, b, s] if !text => [*x, *y, *b, *s],
            _ => return None,
        };
        // A negative blur is invalid, a calculation's clamped to 0.
        let blur = match (blur < 0.0, calculated.get(2)) {
            (true, Some(true)) => 0.0,
            (true, _) => return None,
            (false, _) => blur,
        };
        shadows.push(crate::computed::Shadow {
            // A colour left out is `currentcolor` (D66).
            color: color.unwrap_or(Color::CURRENT.resolve(bases.color)),
            x,
            y,
            blur,
            spread,
            inset,
        });
    }
    (shadows.len() <= MAX_SHADOWS).then_some(shadows)
}

/// `filter` (Filter Effects 1 section 5 and 13, gap batch E, D52):
/// `none`, or filter functions separated by white space, at most
/// [`MAX_FILTERS`]; a `url()` (SVG's filters, G87) is not one here.
fn parse_filter(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Vec<Filter>> {
    use crate::computed::Filter as F;
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for part in split_components(tokens) {
        let [Token::Function(name), arguments @ .., Token::CloseParen] = trim(&part) else {
            return None;
        };
        let name = name.to_ascii_lowercase();
        let args = trim(arguments);
        // An amount: a number or a percentage, never negative (a
        // calculation's clamped to 0), its default when left out; at
        // most 1 where `capped`.
        let amount = |default: f32, capped: bool| -> Option<f32> {
            let v = if args.is_empty() {
                default
            } else {
                match single_token(args) {
                    Some(Token::Number { value, .. }) => *value as f32,
                    Some(Token::Percentage { value }) => *value as f32 / 100.0,
                    _ => match math(args, Want::NumberOrPercentage, bases, None)? {
                        calc::Computed::Value(v) => v.max(0.0) as f32,
                        calc::Computed::Percent(p) => p.max(0.0) as f32 / 100.0,
                        calc::Computed::Expression(_) => return None,
                    },
                }
            };
            if !v.is_finite() || v < 0.0 {
                return None;
            }
            Some(if capped { v.min(1.0) } else { v })
        };
        out.push(match name.as_str() {
            "blur" => F::Blur(if args.is_empty() {
                0.0
            } else {
                length(args, bases, Range::NonNegative)?
            }),
            "brightness" => F::Brightness(amount(1.0, false)?),
            "contrast" => F::Contrast(amount(1.0, false)?),
            "grayscale" => F::Grayscale(amount(1.0, true)?),
            "invert" => F::Invert(amount(1.0, true)?),
            "opacity" => F::Opacity(amount(1.0, true)?),
            "saturate" => F::Saturate(amount(1.0, false)?),
            "sepia" => F::Sepia(amount(1.0, true)?),
            "hue-rotate" => F::HueRotate(if args.is_empty() {
                0.0
            } else {
                angle_degrees(args, bases)?
            }),
            "drop-shadow" => {
                let shadow = parse_shadows(args, bases, false)?;
                let [s] = shadow.as_slice() else {
                    return None;
                };
                let lengths = split_components(args)
                    .iter()
                    .filter(|p| length(p, bases, Range::All).is_some())
                    .count();
                if s.inset || lengths > 3 {
                    return None;
                }
                F::DropShadow {
                    color: s.color,
                    x: s.x,
                    y: s.y,
                    blur: s.blur,
                }
            }
            _ => return None,
        });
    }
    (!out.is_empty() && out.len() <= MAX_FILTERS).then_some(out)
}

/// An `<angle>` in degrees, or `0`, or a math function of one.
fn angle_degrees(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if let Some(calc::Computed::Value(v)) = math(tokens, Want::Angle, bases, None) {
        return Some(v as f32);
    }
    match single_token(tokens)? {
        Token::Number { value, .. } if *value == 0.0 => Some(0.0),
        Token::Dimension { value, unit } => Some(match unit.to_ascii_lowercase().as_str() {
            "deg" => *value as f32,
            "grad" => (*value * 0.9) as f32,
            "rad" => value.to_degrees() as f32,
            "turn" => (*value * 360.0) as f32,
            _ => return None,
        }),
        _ => None,
    }
}

/// The most functions a `filter` takes.
pub const MAX_FILTERS: usize = 32;

/// The most shadows a `box-shadow` takes: each is a blur the backends
/// work out, and a page asking for more is asking for too much.
pub const MAX_SHADOWS: usize = 64;

/// The most words a `color-scheme` keeps.
const MAX_SCHEME_WORDS: usize = 16;

/// Whether an element of `scheme` uses the dark scheme (D49): it names
/// `dark`, and the user prefers it or it names no `light`; `normal` is
/// the page's, its `color-scheme` meta element's.
fn uses_dark(scheme: Option<&str>, resolution: &Resolution<'_>) -> bool {
    let Some(words) = scheme.or(resolution.page_scheme) else {
        return false;
    };
    let has = |name: &str| words.split(' ').any(|w| w.eq_ignore_ascii_case(name));
    if !has("dark") {
        return false;
    }
    if !has("light") {
        return true;
    }
    if let Some(read) = resolution.scheme_read {
        read.set(true);
    }
    resolution.context.media.preferences.color_scheme == ColorScheme::Dark
}

/// What a colour resolves against (gap batch E): the element's
/// `currentcolor`, whether it uses the dark scheme (D49), and its lengths,
/// which a relative length in a colour's component is resolved by (D78).
#[derive(Clone, Copy)]
pub(super) struct Colors<'a> {
    pub(super) current: Color,
    pub(super) dark: bool,
    pub(super) lengths: Option<&'a dyn calc::Lengths>,
}

/// Whether `tokens` mix `currentcolor` in a `color-mix()`, take it as a
/// relative colour's origin or contrast with it (D78): a value kept
/// unresolved for the element where it is used (Color 5 sections 3.1, 4
/// and 6).
fn mixes_current_color(tokens: &[Token]) -> bool {
    tokens.iter().any(|t| match t {
        Token::Function(name) => {
            name.eq_ignore_ascii_case("color-mix") || name.eq_ignore_ascii_case("contrast-color")
        }
        Token::Ident(word) => word.eq_ignore_ascii_case("from"),
        _ => false,
    }) && tokens
        .iter()
        .any(|t| matches!(t, Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor")))
}

/// Keeps `mix` as `style`'s `background-color` mixing `currentcolor`, or
/// none; a style with no other background keeps none.
fn keep_color_mix(style: &mut ComputedStyle, mix: Option<Arc<[Token]>>) {
    if mix.is_some() || style.background().color_mix.is_some() {
        style.background_mut().color_mix = mix;
    }
}

/// The colour `mix`, a value mixing `currentcolor`, is for an element
/// whose `color` is `current`.
pub(crate) fn mix_again(mix: &[Token], current: Color, dark: bool) -> Option<Color> {
    color_value(
        mix,
        Colors {
            current,
            dark,
            lengths: None,
        },
    )
}

/// A colour, `currentcolor` being `colors.current`, the system colours and
/// `light-dark()` by its scheme, in `color-mix()` too (gap batch E).
/// `currentcolor` alone keeps its keyword (D66, G226), resolved for now
/// to `colors.current` and again once the element's `color` is final
/// ([`ComputedStyle::resolve_current_color`]); in a function it is the
/// colour it is.
pub(super) fn color_value(tokens: &[Token], colors: Colors) -> Option<Color> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("currentcolor")) {
        return Some(Color::CURRENT.resolve(colors.current));
    }
    color_in(tokens, Some(colors.current), colors.dark, colors.lengths)
}

/// `color`'s value: its `currentcolor` the parent's colour; `inherit`
/// the parent's, `initial` `CanvasText` by the element's scheme (D49).
/// A relative length in it is the element's (D78).
fn color_of_color(
    tokens: &[Token],
    parent: &ComputedStyle,
    bases: &LengthBases<'_>,
) -> Option<Color> {
    match css_wide_keyword(tokens) {
        Some(CssWide::Initial) => crate::color::system("canvastext", bases.dark),
        Some(_) => Some(parent.color),
        None => color_in(tokens, Some(parent.color), bases.dark, Some(bases)),
    }
}

/// A colour, without an element: no `currentcolor`, the light scheme.
pub(crate) fn parse_color(tokens: &[Token]) -> Option<Color> {
    color_with(tokens, None, false)
}

/// [`color_in`] without an element's lengths, which a relative length in
/// a component then makes invalid.
pub(super) fn color_with(tokens: &[Token], current: Option<Color>, dark: bool) -> Option<Color> {
    color_in(tokens, current, dark, None)
}

/// A colour, `currentcolor` being `current` (none without an element), the
/// system colours and `light-dark()` by the `dark` scheme, a relative
/// length in a component resolved by `lengths`; a relative colour of
/// every function (css-color-5 section 4, D78) from any colour these
/// take.
fn color_in(
    tokens: &[Token],
    current: Option<Color>,
    dark: bool,
    lengths: Option<&dyn calc::Lengths>,
) -> Option<Color> {
    let tokens = trim(tokens);
    if let [Token::Function(name), arguments @ .., Token::CloseParen] = tokens {
        let mut origin = |part: &[Token]| color_in(part, current, dark, lengths);
        if crate::color::is_relative(arguments) {
            return crate::color::relative(name, arguments, &mut origin, lengths);
        }
        let math = |hue_first| crate::color::computed_math(arguments, hue_first, lengths);
        return match name.to_ascii_lowercase().as_str() {
            "rgb" | "rgba" => parse_rgb(&math(false)?),
            "hsl" | "hsla" => parse_hsl(&math(true)?, false),
            "hwb" => parse_hsl(&math(true)?, true),
            "color-mix" => crate::color::parse_mix(arguments, &mut origin, lengths),
            // `contrast-color()` (css-color-5 section 6, D78): white or
            // black, whichever stands out more against the colour.
            "contrast-color" => Some(crate::color::contrast(&origin(arguments)?)),
            // `light-dark()` (Color 5 section 5): two colours, the one
            // of the element's scheme taken.
            "light-dark" => {
                let parts = split_top_level_commas(arguments);
                let [light, darker] = parts.as_slice() else {
                    return None;
                };
                let light = origin(light)?;
                let darker = origin(darker)?;
                Some(if dark { darker } else { light })
            }
            other => crate::color::parse_function(other, arguments, lengths),
        };
    }
    match single_token(tokens)? {
        Token::Hash { value, .. } => parse_hex_color(value),
        Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor") => current,
        Token::Ident(name) => named_color(name).or_else(|| crate::color::system(name, dark)),
        _ => None,
    }
}

/// `tokens` split at their commas outside any function or parentheses.
fn split_top_level_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Function(_) | Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                out.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&tokens[start..]);
    out
}

/// The arguments of `rgb()` or `rgba()`, the same function (css-color-4
/// section 5.1): the legacy syntax, three numbers or three percentages and
/// an optional alpha separated by commas, or the modern one, three
/// numbers or percentages, mixed, and an optional alpha after `/`,
/// separated by white space, `none` as zero. A number is out of 255, a
/// percentage out of 100, each clamped; an alpha a number out of 1 or a
/// percentage. M25: what `getComputedStyle` gives a script to write back.
fn parse_rgb(arguments: &[Token]) -> Option<Color> {
    let legacy = arguments.iter().any(|token| matches!(token, Token::Comma));
    let mut channels: Vec<&Token> = Vec::new();
    let mut alpha = None;
    if legacy {
        for (i, part) in arguments
            .split(|token| matches!(token, Token::Comma))
            .enumerate()
        {
            let value = single_token(part)?;
            match i {
                0..=2 => channels.push(value),
                3 => alpha = Some(value),
                _ => return None,
            }
        }
        // The legacy syntax takes no `none`, and its channels all of one
        // kind.
        let percent = matches!(channels.first(), Some(Token::Percentage { .. }));
        if channels.iter().any(|token| {
            !matches!(
                (percent, token),
                (true, Token::Percentage { .. }) | (false, Token::Number { .. })
            )
        }) || alpha.is_some_and(|token| matches!(token, Token::Ident(_)))
        {
            return None;
        }
    } else {
        let words: Vec<&Token> = arguments
            .iter()
            .filter(|token| !matches!(token, Token::Whitespace))
            .collect();
        match words.as_slice() {
            [r, g, b] => channels.extend([*r, *g, *b]),
            [r, g, b, Token::Delim('/'), a] => {
                channels.extend([*r, *g, *b]);
                alpha = Some(*a);
            }
            _ => return None,
        }
    }
    if channels.len() != 3 {
        return None;
    }
    let channel = |token: &Token| -> Option<f32> {
        let value = match token {
            Token::Number { value, .. } => value / 255.0,
            Token::Percentage { value } => value / 100.0,
            Token::Ident(none) if none.eq_ignore_ascii_case("none") => 0.0,
            _ => return None,
        };
        Some(value.clamp(0.0, 1.0) as f32)
    };
    let a = match alpha {
        None => 1.0,
        Some(Token::Number { value, .. }) => value.clamp(0.0, 1.0),
        Some(Token::Percentage { value }) => (value / 100.0).clamp(0.0, 1.0),
        Some(Token::Ident(none)) if none.eq_ignore_ascii_case("none") => 0.0,
        Some(_) => return None,
    };
    let rgb = [
        channel(channels[0])?,
        channel(channels[1])?,
        channel(channels[2])?,
    ];
    Some(Color::legacy(
        crate::color::Space::Legacy,
        rgb.map(f64::from),
        a,
        nones(&channels, alpha),
    ))
}

/// The bits of [`Color`]'s `missing` for the components and the alpha
/// that are `none`.
fn nones(components: &[&Token], alpha: Option<&Token>) -> u8 {
    let none = |token: &Token| matches!(token, Token::Ident(n) if n.eq_ignore_ascii_case("none"));
    let mut bits = 0;
    for (i, token) in components.iter().enumerate() {
        if none(token) {
            bits |= 1 << i;
        }
    }
    if alpha.is_some_and(none) {
        bits |= 1 << 3;
    }
    bits
}

/// The arguments of `hsl()` or `hsla()`, the same function, or of `hwb()`
/// when `whiteness` (css-color-4 sections 7 and 8; M30, which canvas
/// code writes as often as hexadecimal): a hue, a number of degrees or an
/// angle, and two percentages, separated by commas with an optional
/// alpha (the legacy syntax, which `hwb()` has not), or by white space
/// with an optional alpha after `/`, numbers then taken as percentages
/// and `none` as zero.
fn parse_hsl(arguments: &[Token], whiteness: bool) -> Option<Color> {
    let legacy = arguments.iter().any(|token| matches!(token, Token::Comma));
    let mut parts: Vec<&Token> = Vec::new();
    let mut alpha = None;
    if legacy {
        if whiteness {
            return None;
        }
        for (i, part) in arguments
            .split(|token| matches!(token, Token::Comma))
            .enumerate()
        {
            let value = single_token(part)?;
            match i {
                0..=2 => parts.push(value),
                3 => alpha = Some(value),
                _ => return None,
            }
        }
        // The legacy syntax takes percentages for the two and no `none`.
        if parts.len() != 3
            || parts[1..]
                .iter()
                .any(|token| !matches!(token, Token::Percentage { .. }))
            || alpha.is_some_and(|token| matches!(token, Token::Ident(_)))
            || matches!(parts[0], Token::Ident(_))
        {
            return None;
        }
    } else {
        let words: Vec<&Token> = arguments
            .iter()
            .filter(|token| !matches!(token, Token::Whitespace))
            .collect();
        match words.as_slice() {
            [h, s, l] => parts.extend([*h, *s, *l]),
            [h, s, l, Token::Delim('/'), a] => {
                parts.extend([*h, *s, *l]);
                alpha = Some(*a);
            }
            _ => return None,
        }
    }
    let degrees = match parts[0] {
        Token::Number { value, .. } => *value,
        Token::Dimension { value, unit } => match unit.to_ascii_lowercase().as_str() {
            "deg" => *value,
            "grad" => value * 0.9,
            "rad" => value.to_degrees(),
            "turn" => value * 360.0,
            _ => return None,
        },
        Token::Ident(none) if none.eq_ignore_ascii_case("none") => 0.0,
        _ => return None,
    };
    let fraction = |token: &Token| -> Option<f64> {
        let value = match token {
            Token::Percentage { value } | Token::Number { value, .. } => value / 100.0,
            Token::Ident(none) if none.eq_ignore_ascii_case("none") => 0.0,
            _ => return None,
        };
        Some(value.clamp(0.0, 1.0))
    };
    let (first, second) = (fraction(parts[1])?, fraction(parts[2])?);
    let a = match alpha {
        None => 1.0,
        Some(Token::Number { value, .. }) => value.clamp(0.0, 1.0),
        Some(Token::Percentage { value }) => (value / 100.0).clamp(0.0, 1.0),
        Some(Token::Ident(none)) if none.eq_ignore_ascii_case("none") => 0.0,
        Some(_) => return None,
    };
    // The colour keeps its own components, and which are `none`, for a
    // mix (gap batch E).
    let space = if whiteness {
        crate::color::Space::Hwb
    } else {
        crate::color::Space::Hsl
    };
    Some(Color::legacy(
        space,
        [degrees.rem_euclid(360.0), first * 100.0, second * 100.0],
        a,
        nones(&parts, alpha),
    ))
}

/// A colour written as CSS writes one, with the functions and names the
/// engine knows (M30: for a canvas's styles); `None` for any other text.
pub fn parse_color_text(text: &str) -> Option<Color> {
    let mut tokens = css::tokenize(text);
    // A function the text leaves open is closed at its end, as CSS
    // Syntax 3 closes one at the end of its input (section 5.4.9).
    let open = tokens.iter().fold(0usize, |open, token| match token {
        Token::Function(_) | Token::OpenParen => open + 1,
        Token::CloseParen => open.saturating_sub(1),
        _ => open,
    });
    tokens.extend(std::iter::repeat_n(Token::CloseParen, open));
    parse_color(&tokens)
}

/// `content` (CSS 2.2 §12.2, M16): `normal`, `none`, or one or more
/// of a string, `url()` and the four quote keywords; and after them,
/// css-content-3's `/` and alternative text of strings, accepted and set
/// aside, as nothing reads it. An `attr()` is a substitution function
/// since M21.5, replaced before this reads the value: by a string of the
/// attribute's value, an empty one when there is none, without a type.
/// A `url()` is left out until images (M23). `counter()` and
/// `counters()` (M21.4) are kept as such, for the box tree to write
/// with the counters the box sees; the alternative text takes them too,
/// as css-content-3 says.
fn parse_content(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Content> {
    if let Some(keyword) = single_ident(tokens) {
        if keyword.eq_ignore_ascii_case("normal") {
            return Some(Content::Normal);
        }
        if keyword.eq_ignore_ascii_case("none") {
            return Some(Content::None);
        }
    }
    let mut items = Vec::new();
    // Whether anything came before the `/`, and after it.
    let (mut parts, mut alternative) = (0, None);
    for part in components(tokens) {
        let item = match part {
            [Token::Delim('/')] if parts > 0 && alternative.is_none() => {
                alternative = Some(0);
                continue;
            }
            [Token::String { value, bad: false }] => Some(ContentItem::Text(value.clone())),
            [Token::Function(_), ..] if lists::counter_item(part).is_some() => {
                lists::counter_item(part)
            }
            _ if alternative.is_some() => return None,
            // An image (M23.3), one an `image-set()` with no option this
            // engine shows leaves out.
            part if backgrounds::image(part, bases).is_some() => {
                backgrounds::image(part, bases)?.map(ContentItem::Image)
            }
            [Token::Ident(keyword)] => Some(match keyword.to_ascii_lowercase().as_str() {
                "open-quote" => ContentItem::OpenQuote,
                "close-quote" => ContentItem::CloseQuote,
                "no-open-quote" => ContentItem::NoOpenQuote,
                "no-close-quote" => ContentItem::NoCloseQuote,
                _ => return None,
            }),
            _ => return None,
        };
        match &mut alternative {
            Some(count) => *count += 1,
            None => {
                parts += 1;
                items.extend(item);
            }
        }
    }
    if parts == 0 || alternative == Some(0) {
        return None;
    }
    Some(Content::Items(Arc::from(items)))
}

/// `tokens` without the white space at their ends.
fn trim(mut tokens: &[Token]) -> &[Token] {
    while let [Token::Whitespace, rest @ ..] = tokens {
        tokens = rest;
    }
    while let [rest @ .., Token::Whitespace] = tokens {
        tokens = rest;
    }
    tokens
}

/// `quotes` (CSS 2.2 §12.3.1, css-content-3 for `auto`): `auto`,
/// `none`, or pairs of strings, opening mark then closing mark.
fn parse_quotes(tokens: &[Token]) -> Option<Quotes> {
    if let Some(keyword) = single_ident(tokens) {
        return match keyword.to_ascii_lowercase().as_str() {
            "auto" => Some(Quotes::Auto),
            "none" => Some(Quotes::None),
            _ => None,
        };
    }
    let strings = tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace))
        .map(|token| match token {
            Token::String { value, bad: false } => Some(value.clone()),
            _ => None,
        })
        .collect::<Option<Vec<String>>>()?;
    if strings.is_empty() || strings.len() % 2 != 0 {
        return None;
    }
    let pairs: Vec<(String, String)> = strings
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[open, close]| (open.clone(), close.clone()))
        .collect();
    Some(Quotes::Pairs(Arc::from(pairs)))
}

/// Whether `tokens` are one `<url>`: `url()` with the address bare or a
/// string (Values 4 section 4.5).
fn is_url(tokens: &[Token]) -> bool {
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    match tokens.as_slice() {
        [Token::Url { bad: false, .. }] => true,
        [
            Token::Function(name),
            Token::String { bad: false, .. },
            Token::CloseParen,
        ] => name.eq_ignore_ascii_case("url"),
        _ => false,
    }
}

fn single_token(tokens: &[Token]) -> Option<&Token> {
    let mut it = tokens.iter().filter(|t| !matches!(t, Token::Whitespace));
    let first = it.next()?;
    match it.next() {
        None => Some(first),
        Some(_) => None,
    }
}

/// A `scroll-snap-type`, `scroll-snap-align`, `scroll-snap-stop` or
/// `scroll-initial-target` value as a declaration block keeps it: at its
/// shortest, as its computed value is written (a `proximity` and a
/// second alignment saying the first again left out); `None` for one not
/// valid.
pub fn scroll_snap_value(name: &str, tokens: &[Token]) -> Option<String> {
    let mut style = ComputedStyle::default();
    let parent = ComputedStyle::default();
    let taken = with_default_bases(|bases| {
        apply_declaration(&mut style, name, tokens, bases, &parent, &mut [false; 4])
    });
    taken
        .then(|| crate::serialize::computed_value(&style, name))
        .flatten()
}

/// `scroll-snap-type` (Scroll Snap 1 section 5.1): `none`, or an axis
/// (`pair` among them, Scroll Snap 2's, as the reference browser takes
/// it) and, after it, a strictness, `proximity` when none is given.
fn parse_scroll_snap_type(tokens: &[Token]) -> Option<ScrollSnapType> {
    let words: Option<Vec<String>> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .map(|t| match t {
            Token::Ident(word) => Some(word.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let words = words?;
    let axis = |word: &str| {
        Some(match word {
            "x" => SnapAxis::X,
            "y" => SnapAxis::Y,
            "block" => SnapAxis::Block,
            "inline" => SnapAxis::Inline,
            "both" => SnapAxis::Both,
            "pair" => SnapAxis::Pair,
            _ => return None,
        })
    };
    let strictness = |word: &str| match word {
        "mandatory" => Some(SnapStrictness::Mandatory),
        "proximity" => Some(SnapStrictness::Proximity),
        _ => None,
    };
    match &words[..] {
        [none] if none == "none" => Some(ScrollSnapType::NONE),
        [a] => Some(ScrollSnapType {
            axis: axis(a)?,
            strictness: SnapStrictness::Proximity,
        }),
        [a, s] => Some(ScrollSnapType {
            axis: axis(a)?,
            strictness: strictness(s)?,
        }),
        _ => None,
    }
}

/// `scroll-snap-align` (section 6.3): one or two of `none`, `start`,
/// `end` and `center`, the block axis's then the inline one's, one alone
/// for both.
fn parse_scroll_snap_align(tokens: &[Token]) -> Option<[SnapAlign; 2]> {
    let aligns: Option<Vec<SnapAlign>> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .map(|t| match t {
            Token::Ident(word) => match word.to_ascii_lowercase().as_str() {
                "none" => Some(SnapAlign::None),
                "start" => Some(SnapAlign::Start),
                "end" => Some(SnapAlign::End),
                "center" => Some(SnapAlign::Center),
                _ => None,
            },
            _ => None,
        })
        .collect();
    match aligns?[..] {
        [one] => Some([one, one]),
        [block, inline] => Some([block, inline]),
        _ => None,
    }
}

fn single_ident(tokens: &[Token]) -> Option<&str> {
    match single_token(tokens)? {
        Token::Ident(s) => Some(s.as_str()),
        _ => None,
    }
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (css-color-4 §4.2.1) — any
/// other length, or any non-hex-digit character, is invalid.
fn parse_hex_color(hex: &str) -> Option<Color> {
    let digit = |c: u8| (c as char).to_digit(16);
    let bytes = hex.as_bytes();
    let one = |i: usize| -> Option<f32> { Some(digit(*bytes.get(i)?)? as f32 / 15.0) };
    let two = |i: usize| -> Option<f32> {
        let hi = digit(*bytes.get(i)?)?;
        let lo = digit(*bytes.get(i + 1)?)?;
        Some((hi * 16 + lo) as f32 / 255.0)
    };
    match hex.len() {
        3 => Some(Color::rgba(one(0)?, one(1)?, one(2)?, 1.0)),
        4 => Some(Color::rgba(one(0)?, one(1)?, one(2)?, one(3)?)),
        6 => Some(Color::rgba(two(0)?, two(2)?, two(4)?, 1.0)),
        8 => Some(Color::rgba(two(0)?, two(2)?, two(4)?, two(6)?)),
        _ => None,
    }
}

/// The named colours of CSS Color 4 §6.1 (M17: all 148, where M5 had
/// eighteen), by name in ASCII order for a binary search: the table's
/// RGB values as the standard gives them. `transparent` is the one
/// keyword besides them; `currentcolor` is not a colour here.
const NAMED_COLORS: [(&str, [u8; 3]); 148] = [
    ("aliceblue", [240, 248, 255]),
    ("antiquewhite", [250, 235, 215]),
    ("aqua", [0, 255, 255]),
    ("aquamarine", [127, 255, 212]),
    ("azure", [240, 255, 255]),
    ("beige", [245, 245, 220]),
    ("bisque", [255, 228, 196]),
    ("black", [0, 0, 0]),
    ("blanchedalmond", [255, 235, 205]),
    ("blue", [0, 0, 255]),
    ("blueviolet", [138, 43, 226]),
    ("brown", [165, 42, 42]),
    ("burlywood", [222, 184, 135]),
    ("cadetblue", [95, 158, 160]),
    ("chartreuse", [127, 255, 0]),
    ("chocolate", [210, 105, 30]),
    ("coral", [255, 127, 80]),
    ("cornflowerblue", [100, 149, 237]),
    ("cornsilk", [255, 248, 220]),
    ("crimson", [220, 20, 60]),
    ("cyan", [0, 255, 255]),
    ("darkblue", [0, 0, 139]),
    ("darkcyan", [0, 139, 139]),
    ("darkgoldenrod", [184, 134, 11]),
    ("darkgray", [169, 169, 169]),
    ("darkgreen", [0, 100, 0]),
    ("darkgrey", [169, 169, 169]),
    ("darkkhaki", [189, 183, 107]),
    ("darkmagenta", [139, 0, 139]),
    ("darkolivegreen", [85, 107, 47]),
    ("darkorange", [255, 140, 0]),
    ("darkorchid", [153, 50, 204]),
    ("darkred", [139, 0, 0]),
    ("darksalmon", [233, 150, 122]),
    ("darkseagreen", [143, 188, 143]),
    ("darkslateblue", [72, 61, 139]),
    ("darkslategray", [47, 79, 79]),
    ("darkslategrey", [47, 79, 79]),
    ("darkturquoise", [0, 206, 209]),
    ("darkviolet", [148, 0, 211]),
    ("deeppink", [255, 20, 147]),
    ("deepskyblue", [0, 191, 255]),
    ("dimgray", [105, 105, 105]),
    ("dimgrey", [105, 105, 105]),
    ("dodgerblue", [30, 144, 255]),
    ("firebrick", [178, 34, 34]),
    ("floralwhite", [255, 250, 240]),
    ("forestgreen", [34, 139, 34]),
    ("fuchsia", [255, 0, 255]),
    ("gainsboro", [220, 220, 220]),
    ("ghostwhite", [248, 248, 255]),
    ("gold", [255, 215, 0]),
    ("goldenrod", [218, 165, 32]),
    ("gray", [128, 128, 128]),
    ("green", [0, 128, 0]),
    ("greenyellow", [173, 255, 47]),
    ("grey", [128, 128, 128]),
    ("honeydew", [240, 255, 240]),
    ("hotpink", [255, 105, 180]),
    ("indianred", [205, 92, 92]),
    ("indigo", [75, 0, 130]),
    ("ivory", [255, 255, 240]),
    ("khaki", [240, 230, 140]),
    ("lavender", [230, 230, 250]),
    ("lavenderblush", [255, 240, 245]),
    ("lawngreen", [124, 252, 0]),
    ("lemonchiffon", [255, 250, 205]),
    ("lightblue", [173, 216, 230]),
    ("lightcoral", [240, 128, 128]),
    ("lightcyan", [224, 255, 255]),
    ("lightgoldenrodyellow", [250, 250, 210]),
    ("lightgray", [211, 211, 211]),
    ("lightgreen", [144, 238, 144]),
    ("lightgrey", [211, 211, 211]),
    ("lightpink", [255, 182, 193]),
    ("lightsalmon", [255, 160, 122]),
    ("lightseagreen", [32, 178, 170]),
    ("lightskyblue", [135, 206, 250]),
    ("lightslategray", [119, 136, 153]),
    ("lightslategrey", [119, 136, 153]),
    ("lightsteelblue", [176, 196, 222]),
    ("lightyellow", [255, 255, 224]),
    ("lime", [0, 255, 0]),
    ("limegreen", [50, 205, 50]),
    ("linen", [250, 240, 230]),
    ("magenta", [255, 0, 255]),
    ("maroon", [128, 0, 0]),
    ("mediumaquamarine", [102, 205, 170]),
    ("mediumblue", [0, 0, 205]),
    ("mediumorchid", [186, 85, 211]),
    ("mediumpurple", [147, 112, 219]),
    ("mediumseagreen", [60, 179, 113]),
    ("mediumslateblue", [123, 104, 238]),
    ("mediumspringgreen", [0, 250, 154]),
    ("mediumturquoise", [72, 209, 204]),
    ("mediumvioletred", [199, 21, 133]),
    ("midnightblue", [25, 25, 112]),
    ("mintcream", [245, 255, 250]),
    ("mistyrose", [255, 228, 225]),
    ("moccasin", [255, 228, 181]),
    ("navajowhite", [255, 222, 173]),
    ("navy", [0, 0, 128]),
    ("oldlace", [253, 245, 230]),
    ("olive", [128, 128, 0]),
    ("olivedrab", [107, 142, 35]),
    ("orange", [255, 165, 0]),
    ("orangered", [255, 69, 0]),
    ("orchid", [218, 112, 214]),
    ("palegoldenrod", [238, 232, 170]),
    ("palegreen", [152, 251, 152]),
    ("paleturquoise", [175, 238, 238]),
    ("palevioletred", [219, 112, 147]),
    ("papayawhip", [255, 239, 213]),
    ("peachpuff", [255, 218, 185]),
    ("peru", [205, 133, 63]),
    ("pink", [255, 192, 203]),
    ("plum", [221, 160, 221]),
    ("powderblue", [176, 224, 230]),
    ("purple", [128, 0, 128]),
    ("rebeccapurple", [102, 51, 153]),
    ("red", [255, 0, 0]),
    ("rosybrown", [188, 143, 143]),
    ("royalblue", [65, 105, 225]),
    ("saddlebrown", [139, 69, 19]),
    ("salmon", [250, 128, 114]),
    ("sandybrown", [244, 164, 96]),
    ("seagreen", [46, 139, 87]),
    ("seashell", [255, 245, 238]),
    ("sienna", [160, 82, 45]),
    ("silver", [192, 192, 192]),
    ("skyblue", [135, 206, 235]),
    ("slateblue", [106, 90, 205]),
    ("slategray", [112, 128, 144]),
    ("slategrey", [112, 128, 144]),
    ("snow", [255, 250, 250]),
    ("springgreen", [0, 255, 127]),
    ("steelblue", [70, 130, 180]),
    ("tan", [210, 180, 140]),
    ("teal", [0, 128, 128]),
    ("thistle", [216, 191, 216]),
    ("tomato", [255, 99, 71]),
    ("turquoise", [64, 224, 208]),
    ("violet", [238, 130, 238]),
    ("wheat", [245, 222, 179]),
    ("white", [255, 255, 255]),
    ("whitesmoke", [245, 245, 245]),
    ("yellow", [255, 255, 0]),
    ("yellowgreen", [154, 205, 50]),
];

/// A named colour, in any ASCII case, or `transparent`.
fn named_color(name: &str) -> Option<Color> {
    let name = name.to_ascii_lowercase();
    if name == "transparent" {
        return Some(Color::TRANSPARENT);
    }
    // The reference browser's focus ring (M26.4).
    if name == "-webkit-focus-ring-color" {
        return Some(Color::rgb8(16, 16, 16));
    }
    let at = NAMED_COLORS
        .binary_search_by(|(known, _)| known.cmp(&name.as_str()))
        .ok()?;
    let [r, g, b] = NAMED_COLORS[at].1;
    Some(Color::rgb8(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::{
        AnimationDirection, AnimationFillMode, AnimationPlayState, TextTransform,
        TransitionProperty,
    };
    use crate::selector_matching::{matches_selector, matches_selector_list, parse_selector_list};

    fn sheet(src: &str) -> css::Stylesheet {
        css::parse_stylesheet(src).expect("test stylesheets are valid")
    }

    #[test]
    fn an_attributes_value_never_becomes_a_url() {
        // Values 5's attr() taint (M23.3): directly, through a custom
        // property, and parsed as CSS by `type(*)`.
        let images = |html: &str, css: &str| {
            let sheet = css::parse_stylesheet(css).expect("never fails");
            style_of(html, &[sheet], "div").background().image.clone()
        };
        let html = "<div data-u='a.png' data-v='url(a.png)' data-c='red'>x</div>";
        let none = vec![None];
        assert_eq!(
            images(html, "div { background-image: image-set(attr(data-u)) }"),
            none
        );
        assert_eq!(
            images(
                html,
                "div { --u: attr(data-u); background-image: image-set(var(--u)) }"
            ),
            none
        );
        assert_eq!(
            images(
                html,
                "div { --u: attr(data-u); --w: var(--u); background-image: image-set(var(--w)) }"
            ),
            none
        );
        assert_eq!(
            images(
                html,
                "body { --u: attr(data-u) } div { background-image: image-set(var(--u)) }"
            ),
            none
        );
        // A chain written last to first is followed to its end.
        assert_eq!(
            images(
                html,
                "div { --a: var(--b); --b: var(--c); --c: attr(data-u);
                 background-image: image-set(var(--a)) }"
            ),
            none
        );
        assert_eq!(
            images(html, "div { background-image: attr(data-v type(*)) }"),
            none
        );
        // A string of the sheet's own is a URL as it should be, and an
        // attribute may give what is not one.
        assert_eq!(
            images(
                html,
                "div { --u: \"a.png\"; background-image: image-set(var(--u)) }"
            ),
            vec![Some(crate::computed::Image::Url {
                url: Arc::from("a.png"),
                resolution: 1.0
            })]
        );
        let sheet =
            css::parse_stylesheet("div { background: url(b.png) attr(data-c type(<color>)) }")
                .expect("never fails");
        let div = style_of(html, &[sheet], "div");
        assert_eq!(div.background_color, Color::rgb8(255, 0, 0));
        assert_eq!(div.background().image.len(), 1);
        assert!(
            div.background().image[0].is_some(),
            "the sheet's own URL stands"
        );
        assert_eq!(
            images(
                html,
                "div { --s: image-set(attr(data-u)); background-image: var(--s) }"
            ),
            none
        );
    }

    #[test]
    fn the_border_image_properties_apply_and_border_resets_them() {
        use crate::computed::{BorderImageRepeat, SliceSide};
        let p = |css: &str| {
            let sheet = css::parse_stylesheet(css).expect("never fails");
            style_of("<div><p>x</p></div>", &[sheet], "p")
        };
        let set = p("p { border-image: url(a.png) 10 / 2 round; border-image-slice: 20% fill }");
        let image = set.border_image();
        assert!(image.source.is_some() && image.fill);
        assert_eq!(image.slice, [SliceSide::Percent(20.0); 4]);
        assert_eq!(image.repeat, [BorderImageRepeat::Round; 2]);
        assert!(
            p("p { border-image: url(a.png) } p { border: 1px solid }")
                .border_image
                .is_none()
        );
        assert!(
            p("p { border-image: url(a.png) } p { border-top: 1px solid }")
                .border_image
                .is_some(),
            "a side's shorthand leaves it"
        );
        assert!(p("p { border-image: none }").border_image.is_none());
        // Not inherited, but taken when asked for.
        assert!(p("div { border-image: url(a.png) }").border_image.is_none());
        let inherited = p("div { border-image: url(a.png) 7 } p { border-image-slice: inherit }");
        assert_eq!(inherited.border_image().slice, [SliceSide::Number(7.0); 4]);
        assert!(inherited.border_image().source.is_none());
    }

    #[test]
    fn the_background_shorthand_sets_the_colour_and_resets_it() {
        // M17: a later shorthand without a colour resets an earlier
        // `background-color`; since M23.3 it sets the layers too, and one
        // the engine cannot honour is dropped whole.
        let colour = |css: &str| {
            let sheet = css::parse_stylesheet(css).expect("never fails");
            style_of("<p>x</p>", &[sheet], "p").background_color
        };
        assert_eq!(colour("p { background: green }"), Color::rgb8(0, 128, 0));
        assert_eq!(
            colour("p { background-color: red; background: none }"),
            ComputedStyle::default().background_color
        );
        assert_eq!(
            colour("p { background: green; background: url(a.png) red }"),
            Color::rgb8(255, 0, 0)
        );
        assert_eq!(
            colour("p { background: green; background: url(a.png) url(b.png) }"),
            Color::rgb8(0, 128, 0)
        );
        let sheet = css::parse_stylesheet(
            "p { background-image: url(a.png); background-position: right; background: blue }",
        )
        .expect("never fails");
        let p = style_of("<p>x</p>", &[sheet], "p");
        assert!(p.background.is_none(), "the shorthand resets the layers");
        let sheet =
            css::parse_stylesheet("p { background: url(a.png) } p { background-repeat: inherit }")
                .expect("never fails");
        let p = style_of("<p>x</p>", &[sheet], "p");
        assert_eq!(p.background().image.len(), 1);
        assert_eq!(
            p.background().repeat,
            ComputedStyle::default().background().repeat
        );
    }

    #[test]
    fn supports_answers_what_the_cascade_does_with_a_declaration() {
        // M17: the cascade's own parsers, every property and alias it
        // knows, the CSS-wide keywords; nothing it would drop.
        let supports = |name: &str, value: &str| {
            let declarations =
                css::parse_style_attribute(&format!("{name}: {value}")).expect("never fails");
            let [d] = declarations.as_slice() else {
                panic!("one declaration in {name}: {value}");
            };
            super::supports(&d.name, &d.value)
        };
        for (name, value) in [
            ("display", "flex"),
            ("color", "rgb(0 0 0 / 50%)"),
            ("display", "-webkit-inline-flex"),
            ("width", "10px"),
            ("width", "fit-content"),
            ("margin", "auto 5%"),
            ("border", "1px solid lightblue"),
            ("-webkit-flex", "1"),
            ("flex-flow", "column wrap"),
            ("place-items", "center"),
            ("gap", "10px 5%"),
            ("font-size", "12px"),
            ("font-size", "150%"),
            ("color", "inherit"),
            ("align-self", "unset"),
            ("display", "grid"),
            ("display", "inline-grid"),
            (
                "grid-template-columns",
                "repeat(auto-fill, minmax(10em, 1fr))",
            ),
            ("grid-template", "'a b' 1fr / auto 1fr"),
            ("grid", "auto-flow dense / 10px"),
            ("grid-area", "a / 2 / span 3"),
            ("justify-items", "legacy center"),
            ("justify-self", "safe right"),
            ("grid-gap", "1px"),
            ("grid-row-gap", "inherit"),
            ("transform", "none"),
            ("transform", "translate(10px, 5%) rotate(45deg) scale(1.5)"),
            (
                "transform",
                "perspective(500px) rotateY(30deg) matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)",
            ),
            ("-webkit-transform", "skewX(10deg)"),
            ("transform-origin", "left top 10px"),
            ("-webkit-transform-origin", "bottom"),
            ("transform-box", "content-box"),
            ("translate", "10px 20%"),
            ("rotate", "x 45deg"),
            ("scale", "2 50%"),
            ("transform-style", "preserve-3d"),
            ("-webkit-transform-style", "flat"),
            ("perspective", "none"),
            ("-webkit-perspective", "800px"),
            ("perspective-origin", "right 10px bottom 20px"),
            ("-webkit-perspective-origin", "25% 75%"),
            ("backface-visibility", "hidden"),
            ("-webkit-backface-visibility", "visible"),
            ("opacity", "0.5"),
            ("opacity", "50%"),
            ("will-change", "transform, opacity"),
            ("will-change", "scroll-position"),
            ("will-change", "left"),
            ("transform", "inherit"),
            ("opacity", "unset"),
        ] {
            assert!(supports(name, value), "{name}: {value}");
        }
        for (name, value) in [
            ("transform", "rotate(45)"),
            ("transform", "translate(10px 20px)"),
            ("transform", "bogus(1)"),
            ("transform-origin", "top 50px"),
            ("transform-box", "padding-box"),
            ("rotate", "0"),
            ("scale", "1px"),
            ("perspective", "-1px"),
            ("perspective", "500"),
            ("perspective-origin", "right 10px bottom"),
            ("opacity", "1px"),
            ("will-change", "none"),
            ("will-change", "transform, auto"),
        ] {
            assert!(!supports(name, value), "{name}: {value}");
        }
        for (name, value) in [
            ("background", "green"),
            ("background", "none"),
            ("background", "none lightblue"),
            // M21.2: the math functions, a negative width among them,
            // which is clamped, not invalid.
            ("width", "calc(10px + 1em)"),
            ("width", "calc(10px - 20px)"),
            ("margin", "min(1px, 2%) clamp(1px, 2em, 3px)"),
            ("z-index", "calc(2.5)"),
            ("opacity", "calc(100% / 3)"),
            (
                "transform",
                "rotate(calc(1turn / 4)) translate(calc(50% - 1px))",
            ),
            (
                "grid-template-columns",
                "repeat(calc(2), minmax(calc(10px * 2), 1fr))",
            ),
        ] {
            assert!(supports(name, value), "{name}: {value}");
        }
        for (name, value) in [
            ("background", "url(a.png) green"),
            ("background", "left top green"),
            ("background-image", "image-set(\"a.png\" 2x)"),
        ] {
            assert!(supports(name, value), "{name}: {value}");
        }
        for (name, value) in [
            ("background", "green, red"),
            ("background", "green red"),
            ("background", "url(a.png) url(b.png)"),
        ] {
            assert!(!supports(name, value), "{name}: {value}");
        }
        for (name, value) in [
            ("grid-template-columns", "subgrid"),
            ("grid-template-rows", "masonry"),
            ("display", "grid-lanes"),
            ("display", "-webkit-box"),
            ("width", "-5px"),
            ("width", "calc(10px + 1s)"),
            ("width", "sin(1deg)"),
            ("color", "lab(0% 0 0 0)"),
            ("writing-mode", "vertical-rl"),
            ("bogus", "inherit"),
            ("font-size", "-1px"),
            ("flex-grow", "-1"),
            ("justify-content", "baseline"),
            ("border", "1px solid red 2px"),
        ] {
            assert!(!supports(name, value), "{name}: {value}");
        }
    }

    /// Runs `f` with the length bases of a 16px element whose root is
    /// 16px, in an 800 by 600 viewport, with no font.
    pub(super) fn with_bases<R>(f: impl FnOnce(&LengthBases<'_>) -> R) -> R {
        let context = CascadeContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            fonts: None,
            language: None,
            media: Media::screen(800.0, 600.0),
            animations: None,
            inline_styles: None,
            frame_margins: FrameMargins::default(),
            top_layer: None,
        };
        let viewport_used = Cell::new(false);
        let substitutions = Substitutions::new(0);
        let resolution = Resolution {
            rem: 16.0,
            root: false,
            page_scheme: None,
            context: &context,
            viewport_used: &viewport_used,
            fonts_used: None,
            root_font: None,
            root_size: 16.0,
            scheme_read: None,
            attributes: &[],
            substitutions: &substitutions,
            containers: None,
            units_from: None,
            containers_used: &viewport_used,
        };
        f(&LengthBases {
            em: 16.0,
            font: crate::computed::initial_font(),
            line: (crate::computed::initial_font(), 16.0),
            resolution: &resolution,
            color: Color::BLACK,
            dark: false,
        })
    }

    /// Styles `html` and returns the computed style of the element found
    /// by `selector` (the first match, in document order).
    fn style_of(html: &str, sheets: &[css::Stylesheet], selector: &str) -> ComputedStyle {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let styles = cascade_document(&document, sheets, &CascadeContext::default());
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        document
            .subtree(document.root())
            .find(|&id| matches_selector(&document, id, &list.0[0]))
            .and_then(|id| styles.get(id).cloned())
            .unwrap_or_else(|| panic!("no element matched {selector:?}"))
    }

    /// Styles `html` and returns the style of the pseudo-element `pseudo`
    /// of the first element `selector` matches, if some rule styles it.
    fn pseudo_of(
        html: &str,
        sheets: &[css::Stylesheet],
        selector: &str,
        pseudo: PseudoElement,
    ) -> Option<ComputedStyle> {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let styles = cascade_document(&document, sheets, &CascadeContext::default());
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        let element = document
            .subtree(document.root())
            .find(|&id| matches_selector(&document, id, &list.0[0]))
            .unwrap_or_else(|| panic!("no element matched {selector:?}"));
        styles.pseudo(element, pseudo).cloned()
    }

    /// The content of `p::before` with `declaration` in `p`, from a
    /// rule that also gives it the content "before".
    fn content(html: &str, declaration: &str) -> Content {
        let sheets = [sheet(&format!(
            "p::before {{ content: 'before' }} p::before {{ content: {declaration} }}"
        ))];
        pseudo_of(html, &sheets, "p", PseudoElement::Before)
            .expect("styled")
            .content
    }

    fn items(items: &[ContentItem]) -> Content {
        Content::Items(Arc::from(items.to_vec()))
    }

    fn text(text: &str) -> ContentItem {
        ContentItem::Text(text.to_owned())
    }

    /// The first element named `tag`, in tree order.
    fn first(document: &Document, tag: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&id| document.tag(id) == Some(tag))
            .unwrap_or_else(|| panic!("no {tag}"))
    }

    #[test]
    fn styles_are_kept_by_node_and_answer_nothing_for_text_detached_or_stale_nodes() {
        let node = html::parse_document("<p class=a>x</p><p>y</p>").expect("never fails");
        let mut document = Document::from_node(&node);
        let sheet = css::parse_stylesheet(".a { color: #ff0000 }").unwrap();
        let styles = cascade_document(&document, &[sheet], &CascadeContext::default());
        assert_eq!(styles.get(document.root()), Some(&ComputedStyle::default()));
        let paragraphs: Vec<NodeId> = document
            .subtree(document.root())
            .filter(|&id| document.tag(id) == Some("p"))
            .collect();
        assert_eq!(
            styles.get(paragraphs[0]).unwrap().color,
            Color::rgb8(255, 0, 0)
        );
        assert_ne!(
            styles.get(paragraphs[1]).unwrap().color,
            Color::rgb8(255, 0, 0)
        );
        let text = document.first_child(paragraphs[0]).unwrap();
        assert_eq!(
            styles.get(text),
            None,
            "a text node has no style of its own"
        );
        // A node created after the cascade has none, nor does a new node
        // in the slot of one freed since: the styles are a record of the
        // nodes the cascade saw, and a slot's generation tells them apart.
        let late = document.create_element("p").unwrap();
        assert_eq!(styles.get(late), None);
        let body = document.parent(paragraphs[1]).unwrap();
        document.remove_child(body, paragraphs[1]).unwrap();
        document.free(paragraphs[1]).unwrap();
        let reused = [
            document.create_element("p").unwrap(),
            document.create_element("p").unwrap(),
        ]
        .into_iter()
        .find(|id| id.index() == paragraphs[1].index())
        .expect("the freed slot is reused");
        assert_eq!(styles.get(reused), None, "a new node in a styled slot");
    }

    /// Wraps four plain pixel values as `MarginEdges` (test convenience).
    fn px4(edges: [f32; 4]) -> MarginEdges {
        edges.map(LengthPercentageAuto::Px)
    }

    #[test]
    fn ua_stylesheet_parses_and_applies_with_no_author_sheets() {
        let s = style_of("<html><body><p>x</p></body></html>", &[], "p");
        assert_eq!(s.display, Display::Block); // from ua.css
        assert_eq!(s.margin, px4([16.0, 0.0, 16.0, 0.0]));
    }

    #[test]
    fn author_beats_ua_at_equal_specificity_tier() {
        // ua.css sets `p { margin: 16px 0; }`; author overrides fully.
        let sheets = [sheet("p { margin: 5px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.margin, px4([5.0; 4]));
    }

    #[test]
    fn specificity_breaks_ties_within_author_normal() {
        let sheets = [sheet("p { color: red; } p.a { color: blue; }")];
        let s = style_of("<p class='a'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn later_source_order_wins_at_equal_specificity() {
        let sheets = [sheet("p { color: red; } p { color: blue; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn low_specificity_important_beats_high_specificity_normal() {
        let sheets = [sheet("p { color: red !important; } p#x { color: blue; }")];
        let s = style_of("<p id='x'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn author_normal_beats_ua_normal() {
        // The UA sheet makes `p` a block; a normal author rule overrides
        // it (the opposite of the `!important` case below).
        let sheets = [sheet("p { display: inline; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.display, Display::Inline);
    }

    #[test]
    fn rules_of_svgs_subjects_alone_are_marked_and_still_apply_to_svg() {
        use crate::serialize::computed_value;
        // M32: a rule whose every selector's subject is in SVG's
        // namespace is tried against SVG elements alone (T4.3 found the
        // user agent's SVG rules tried against every element).
        let author = [sheet(
            "@namespace svg url(http://www.w3.org/2000/svg);
             svg|rect, svg|g > svg|circle { stroke-width: 7 }
             svg|rect, p { color: red }
             rect { color: blue }
             svg|*:not(svg|svg) p { color: green }",
        )];
        let mut features = SelectorFeatures::default();
        let set = RuleSet::new(&author, &mut features);
        let marked: Vec<bool> = set
            .in_force(&author, &[])
            .iter()
            .map(|r| r.svg_only)
            .collect();
        assert_eq!(marked, [true, false, false, false]);
        let html = "<p id=p>x</p><svg><rect id=r></rect><g><circle id=c></circle></g></svg>";
        let r = style_of(html, &author, "#r");
        assert_eq!(computed_value(&r, "stroke-width").unwrap(), "7px");
        let c = style_of(html, &author, "#c");
        assert_eq!(computed_value(&c, "stroke-width").unwrap(), "7px");
        assert_eq!(style_of(html, &author, "#p").color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn ua_important_beats_author_important() {
        // css-cascade-5 §6.1: UA `!important` is the strongest tier of
        // all. `assets/ua.css` uses none, so a UA sheet of the test's own
        // stands in for the built-in one.
        let ua = sheet("p { color: red !important; display: block; }");
        let author = [sheet(
            "p { color: blue !important; display: inline !important; }",
        )];
        let document = Document::from_node(&html::parse_document("<p>x</p>").expect("never fails"));
        let ua = [ua];
        let mut features = SelectorFeatures::default();
        let ua_set = RuleSet::new(&ua, &mut features);
        let author_set = RuleSet::new(&author, &mut features);
        let context = CascadeContext::default();
        let styles = Cascader::new(
            &document,
            ua_set.in_force(&ua, &[]),
            author_set.in_force(&author, &[]),
            &context,
            Vec::new(),
            0,
        )
        .cascade(None);
        let p = styles.get(first(&document, "p")).expect("styled");
        assert_eq!(p.color, Color::rgb8(255, 0, 0));
        // A normal UA declaration still loses to an important author one.
        assert_eq!(p.display, Display::Inline);
    }

    #[test]
    fn css_wide_keywords_resolve_against_parent_or_initial() {
        let sheets = [sheet(
            "div { color: red; margin: 5px; }
             a { color: inherit; }
             p { margin: initial; display: unset; }
             span { color: unset; }
             b { color: initial; }",
        )];
        let html = "<div><a href=x>link</a><p>para</p><span>s</span><b>bold</b></div>";
        // `a` is UA blue; `inherit` takes the div's red instead.
        assert_eq!(style_of(html, &sheets, "a").color, Color::rgb8(255, 0, 0));
        // `p`'s UA margin (16px 0) resets to the initial 0.
        assert_eq!(style_of(html, &sheets, "p").margin, px4([0.0; 4]));
        // `display` is not inherited, so `unset` is `initial` (inline).
        assert_eq!(style_of(html, &sheets, "p").display, Display::Inline);
        // `color` is inherited, so `unset` is `inherit` (red).
        assert_eq!(
            style_of(html, &sheets, "span").color,
            Color::rgb8(255, 0, 0)
        );
        // `initial` for `color` is black, whatever the parent says.
        assert_eq!(style_of(html, &sheets, "b").color, Color::BLACK);
        // `font-size: inherit` through the first pass.
        let sheets = [sheet("div { font-size: 30px; } p { font-size: inherit; }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").font_size_px,
            30.0
        );
        // A `font-variant` longhand's `initial` is its own, not what the
        // element inherited.
        let sheets = [sheet(
            "div { font-variant-caps: small-caps; } p { font-variant-caps: initial; }",
        )];
        let html = "<div><p>x</p><a>y</a></div>";
        assert_eq!(style_of(html, &sheets, "p").font().variant.caps, "normal");
        assert_eq!(
            style_of(html, &sheets, "a").font().variant.caps,
            "small-caps"
        );
    }

    #[test]
    fn an_empty_border_value_is_ignored() {
        let sheets = [sheet("p { border: 1px solid red; border: ; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [1.0; 4]);
        assert_eq!(s.border_style, [BorderStyle::Solid; 4]);
        let sheets = [sheet("p { border: 1px solid red; border: !important; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [1.0; 4]);
    }

    #[test]
    fn lengths_are_finite_bounded_and_non_negative_where_the_spec_says() {
        let sheets = [sheet(
            "p { margin: 0; font-size: 1e39px; padding: -5px; border-width: -1px; width: 1e30px;
                 margin-top: -1e39px; letter-spacing: 1e20px; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.font_size_px, 16.0, "an infinite font-size is ignored");
        assert_eq!(
            s.padding,
            [const { LengthPercentage::Px(0.0) }; 4],
            "negative padding is ignored"
        );
        assert_eq!(
            s.border_width, [3.0; 4],
            "negative border-width is ignored: medium stays"
        );
        assert_eq!(s.width, Size::Px(MAX_LENGTH_PX));
        assert_eq!(
            s.margin[TOP],
            LengthPercentageAuto::Px(0.0),
            "an infinite margin is ignored"
        );
        assert_eq!(s.letter_spacing, MAX_LENGTH_PX);
        let sheets = [sheet("p { font-size: -16px; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").font_size_px, 16.0);
        // A percentage is kept as written, or as a math function makes
        // it, and is a length in range once it is resolved; a math
        // function's `NaN` is 0 (M21), its infinity the largest a
        // percentage holds (D78).
        let sheets = [sheet(
            "p { width: 1e38%; padding-left: calc(1% * infinity); margin-left: calc(1% * NaN); }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.width, Size::Percent(1e38));
        assert_eq!(
            s.width.length_percentage().map(|w| w.resolve(800.0)),
            Some(MAX_LENGTH_PX)
        );
        assert_eq!(s.padding[LEFT], LengthPercentage::Percent(f32::MAX));
        assert_eq!(s.padding[LEFT].resolve(800.0), MAX_LENGTH_PX);
        assert_eq!(s.padding[LEFT].resolve(0.0), 0.0);
        assert_eq!(s.margin[LEFT], LengthPercentageAuto::Percent(0.0));
    }

    #[test]
    fn inline_style_beats_any_selector_matched_normal_rule() {
        let sheets = [sheet("#x { color: blue; }")]; // higher specificity than any selector could beat via inline... except inline always wins anyway
        let s = style_of("<p id='x' style='color: red'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn author_important_beats_inline_normal() {
        let sheets = [sheet("p { color: blue !important; }")];
        let s = style_of("<p style='color: red'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn inline_important_beats_author_important() {
        let sheets = [sheet("p { color: blue !important; }")];
        let s = style_of("<p style='color: red !important'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn color_and_font_size_inherit_but_margin_does_not() {
        let sheets = [sheet("div { color: red; font-size: 20px; margin: 10px; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
        assert_eq!(s.font_size_px, 20.0);
        assert_eq!(s.margin, px4([0.0; 4])); // initial, not inherited
    }

    #[test]
    fn em_on_font_size_resolves_against_parent() {
        let sheets = [sheet("div { font-size: 20px; } span { font-size: 2em; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.font_size_px, 40.0);
    }

    #[test]
    fn em_on_other_properties_resolves_against_self() {
        let sheets = [sheet("p { font-size: 10px; margin: 2em; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.margin, px4([20.0; 4]));
    }

    #[test]
    fn box_shorthand_expands_one_to_four_values() {
        // `padding`/`border-width`'s shorthand (`margin`'s own, which
        // additionally allows `auto` per value, is tested separately
        // below).
        assert_eq!(
            with_bases(|b| parse_border_width_shorthand(&css::tokenize("10px"), b)),
            Some([10.0; 4])
        );
        assert_eq!(
            with_bases(|b| parse_border_width_shorthand(&css::tokenize("10px 20px"), b)),
            Some([10.0, 20.0, 10.0, 20.0])
        );
        assert_eq!(
            with_bases(|b| parse_border_width_shorthand(&css::tokenize("10px 20px 30px"), b)),
            Some([10.0, 20.0, 30.0, 20.0])
        );
        assert_eq!(
            with_bases(|b| parse_border_width_shorthand(&css::tokenize("10px 20px 30px 40px"), b)),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(
            with_bases(|b| parse_border_width_shorthand(
                &css::tokenize("10px 20px 30px 40px 50px"),
                b
            )),
            None
        );
    }

    #[test]
    fn margin_shorthand_expands_and_allows_auto_per_value() {
        use LengthPercentageAuto::{Auto, Px};
        assert_eq!(
            with_bases(|b| parse_margin_shorthand(&css::tokenize("10px auto"), b)),
            Some([Px(10.0), Auto, Px(10.0), Auto])
        );
        assert_eq!(
            with_bases(|b| parse_margin_shorthand(&css::tokenize("0 auto"), b)),
            Some([Px(0.0), Auto, Px(0.0), Auto])
        );
    }

    #[test]
    fn margin_auto_reaches_computed_style() {
        let sheets = [sheet("p { margin: 0 auto; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.margin[LEFT], LengthPercentageAuto::Auto);
        assert_eq!(s.margin[RIGHT], LengthPercentageAuto::Auto);
        assert_eq!(s.margin[TOP], LengthPercentageAuto::Px(0.0));
    }

    #[test]
    fn text_align_parses_and_inherits() {
        let sheets = [sheet("div { text-align: center; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.text_align, TextAlign::Center, "inherited from div");
    }

    #[test]
    fn text_align_unknown_value_is_ignored() {
        let sheets = [sheet("p { text-align: middle; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.text_align, TextAlign::Start); // initial value, untouched
    }

    #[test]
    fn text_align_sets_its_longhands_and_matches_the_parent() {
        // css-text-3 §7 (M21.7).
        let of = |css_src: &str, selector: &str| {
            let s = style_of(
                "<div dir=rtl><p>x</p></div><ol><li>y</li></ol>",
                &[sheet(css_src)],
                selector,
            );
            (s.text_align, s.text_align_last, s.text_justify)
        };
        let auto = TextAlignLast::Auto;
        let justify = TextAlignLast::Align(TextAlign::Justify);
        for (value, expected) in [
            ("start", TextAlign::Start),
            ("END", TextAlign::End),
            ("left", TextAlign::Left),
            ("right", TextAlign::Right),
            ("center", TextAlign::Center),
            ("justify", TextAlign::Justify),
        ] {
            assert_eq!(
                of(
                    &format!("p {{ text-align-last: right; text-align: {value} }}"),
                    "p"
                ),
                (expected, auto, TextJustify::Auto),
                "{value}"
            );
        }
        assert_eq!(
            of("p { text-align: justify-all }", "p"),
            (TextAlign::Justify, justify, TextJustify::Auto)
        );
        // `match-parent`: the parent's, its start the right in a
        // right-to-left parent; the list item's is the user agent's.
        assert_eq!(
            of("p { text-align: match-parent }", "p").0,
            TextAlign::Right
        );
        assert_eq!(
            of(
                "div { text-align: end; text-align-last: start } p { text-align: match-parent }",
                "p"
            ),
            (
                TextAlign::Left,
                TextAlignLast::Align(TextAlign::Right),
                TextJustify::Auto
            )
        );
        assert_eq!(of("", "li").0, TextAlign::Left);
        // The longhands alone, and `text-justify`.
        assert_eq!(
            of(
                "p { text-align-all: center; text-align-last: justify; text-justify: distribute }",
                "p"
            ),
            (TextAlign::Center, justify, TextJustify::InterCharacter)
        );
        assert_eq!(
            of("p { text-align-all: justify-all; text-justify: none }", "p"),
            (TextAlign::Start, auto, TextJustify::None)
        );
        // Inherited, all three.
        assert_eq!(
            of(
                "div { text-align: justify-all; text-justify: inter-word }",
                "p"
            ),
            (TextAlign::Justify, justify, TextJustify::InterWord)
        );
    }

    #[test]
    fn text_transform_parses_and_inherits() {
        let sheets = [sheet("div { text-transform: uppercase; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn positioning_properties_parse_and_are_not_inherited() {
        let html = "<div><p>x</p></div>";
        let sheets = [sheet(
            "div { float: left; clear: both; position: relative; top: 10px; right: 5%;
                   bottom: auto; left: 2em; z-index: 3; }",
        )];
        let div = style_of(html, &sheets, "div");
        assert_eq!(div.float, Float::Left);
        assert_eq!(div.clear, Clear::Both);
        assert_eq!(div.position, Position::Relative);
        assert_eq!(div.inset[TOP], LengthPercentageAuto::Px(10.0));
        assert_eq!(div.inset[RIGHT], LengthPercentageAuto::Percent(5.0));
        assert_eq!(div.inset[BOTTOM], LengthPercentageAuto::Auto);
        assert_eq!(div.inset[LEFT], LengthPercentageAuto::Px(32.0));
        assert_eq!(div.z_index, Some(3));
        let p = style_of(html, &sheets, "p");
        assert_eq!(p.float, Float::None);
        assert_eq!(p.clear, Clear::None);
        assert_eq!(p.position, Position::Static);
        assert_eq!(p.inset, [const { LengthPercentageAuto::Auto }; 4]);
        assert_eq!(p.z_index, None);
        // Every keyword, whatever its case; an unknown one leaves the
        // value alone; `z-index` takes an integer or `auto`, nothing
        // else.
        for (value, expected) in [
            ("none", Position::Static),
            ("RELATIVE", Position::Relative),
            ("absolute", Position::Absolute),
            ("fixed", Position::Fixed),
            ("sticky", Position::Sticky),
            ("stuck", Position::Static),
        ] {
            let sheets = [sheet(&format!("p {{ position: {value}; }}"))];
            assert_eq!(style_of(html, &sheets, "p").position, expected, "{value}");
        }
        for (value, expected) in [
            ("none", Clear::None),
            ("LEFT", Clear::Left),
            ("right", Clear::Right),
            ("both", Clear::Both),
            ("all", Clear::None),
        ] {
            let sheets = [sheet(&format!("p {{ clear: {value}; }}"))];
            assert_eq!(style_of(html, &sheets, "p").clear, expected, "{value}");
        }
        for (value, expected) in [
            ("auto", None),
            ("-1", Some(-1)),
            ("0", Some(0)),
            ("2147483647", Some(i32::MAX)),
            // Beyond an `i32`, clamped (css-values-4 §5); `1e10` is a
            // number, not an integer.
            ("2147483648", Some(i32::MAX)),
            ("-17014118346", Some(i32::MIN)),
            ("1.5", None),
            ("1e10", None),
            ("10px", None),
        ] {
            let sheets = [sheet(&format!("p {{ z-index: {value}; }}"))];
            assert_eq!(style_of(html, &sheets, "p").z_index, expected, "{value}");
        }
        // The css-wide keywords.
        let sheets = [sheet(
            "div { position: absolute; top: 4px; z-index: 2; }
             p { position: inherit; top: inherit; z-index: inherit; }",
        )];
        let p = style_of(html, &sheets, "p");
        assert_eq!(p.position, Position::Absolute);
        assert_eq!(p.inset[TOP], LengthPercentageAuto::Px(4.0));
        assert_eq!(p.z_index, Some(2));
        let sheets = [sheet(
            "p { float: right; float: unset; clear: left; clear: initial; }",
        )];
        let p = style_of(html, &sheets, "p");
        assert_eq!((p.float, p.clear), (Float::None, Clear::None));
    }

    #[test]
    fn floats_and_absolutely_positioned_boxes_follow_css2_section_9_7() {
        // An absolutely positioned box does not float; a float or an
        // absolutely positioned box that is inline-level is a block.
        let html = "<div><span>x</span></div>";
        let span = |css: &str| style_of(html, &[sheet(css)], "span");
        let s = span("span { float: left; }");
        assert_eq!((s.float, s.display), (Float::Left, Display::Block));
        let s = span("span { position: absolute; float: right; }");
        assert_eq!((s.float, s.display), (Float::None, Display::Block));
        let s = span("span { position: fixed; display: inline-block; }");
        assert_eq!(s.display, Display::Block);
        // A relatively positioned inline stays inline; a float that is
        // `display: none` stays none.
        let s = span("span { position: relative; }");
        assert_eq!(s.display, Display::Inline);
        let s = span("span { float: left; display: none; }");
        assert_eq!(s.display, Display::None);
        // §9.7's table for the table values (M12): an inline table
        // becomes a table, a table-internal box a block, a table stays
        // a table, and none of it applies to a box in flow.
        let s = span("span { float: left; display: inline-table; }");
        assert_eq!(s.display, Display::Table);
        let s = span("span { position: absolute; display: table-cell; }");
        assert_eq!(s.display, Display::Block);
        let s = span("span { float: right; display: table-row-group; }");
        assert_eq!(s.display, Display::Block);
        let s = span("span { float: right; display: table; }");
        assert_eq!(s.display, Display::Table);
        let s = span("span { position: relative; display: table-cell; }");
        assert_eq!(s.display, Display::TableCell);
    }

    #[test]
    fn overflow_parses_its_five_keywords_and_is_not_inherited() {
        for (value, expected) in [
            ("visible", Overflow::Visible),
            ("hidden", Overflow::Hidden),
            ("scroll", Overflow::Scroll),
            ("auto", Overflow::Auto),
            ("clip", Overflow::Clip),
            ("HIDDEN", Overflow::Hidden),
        ] {
            let sheets = [sheet(&format!("div {{ overflow: {value}; }}"))];
            assert_eq!(
                style_of("<div><p>x</p></div>", &sheets, "div").overflow(),
                expected,
                "{value}"
            );
            assert_eq!(
                style_of("<div><p>x</p></div>", &sheets, "p").overflow(),
                Overflow::Visible,
                "overflow is not inherited ({value})"
            );
        }
        // An unknown keyword leaves the initial value; two are the axes',
        // `visible` beside a scrolling axis computing to `auto` and
        // `clip` to `hidden`.
        let sheets = [sheet(
            "div { overflow: clipped; } p { overflow: hidden visible; }",
        )];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "div").overflow(),
            Overflow::Visible
        );
        let p = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(
            (p.overflow_x, p.overflow_y),
            (Overflow::Hidden, Overflow::Auto)
        );
        assert_eq!(p.overflow(), Overflow::Auto);
        // A clip beside a scrolling axis stays: a single-axis scroll
        // container, as the reference browser has it.
        let sheets = [sheet("p { overflow-y: scroll; overflow-x: clip }")];
        let p = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(
            (p.overflow_x, p.overflow_y),
            (Overflow::Clip, Overflow::Scroll)
        );
        let sheets = [sheet("p { overflow-y: clip }")];
        let p = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(
            (p.overflow_x, p.overflow_y),
            (Overflow::Visible, Overflow::Clip)
        );
        assert_eq!(p.overflow(), Overflow::Clip);
        // `inherit` reads the parent, `initial`/`unset` the initial value.
        let sheets = [sheet("div { overflow: hidden; } p { overflow: inherit; }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").overflow(),
            Overflow::Hidden
        );
        let sheets = [sheet(
            "div { overflow: hidden; } p { overflow: hidden; } p { overflow: unset; }",
        )];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").overflow(),
            Overflow::Visible
        );
    }

    #[test]
    fn box_sizing_and_the_min_and_max_sizes_parse() {
        let sheets = [sheet(
            "p { box-sizing: border-box; min-width: 10px; max-width: 50%; min-height: 2em;
                 max-height: none; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.box_sizing, BoxSizing::BorderBox);
        assert_eq!(s.min_width, Size::Px(10.0));
        assert_eq!(s.max_width, MaxSize::Percent(50.0));
        assert_eq!(s.min_height, Size::Px(32.0));
        assert_eq!(s.max_height, MaxSize::None);
        // Negative sizes and unknown keywords are ignored; `max-width:
        // none` undoes an earlier value; the properties are not
        // inherited and take the css-wide keywords.
        let sheets = [sheet(
            "div { max-width: 100px; box-sizing: border-box; }
             p { min-width: -1px; max-height: -5%; box-sizing: padding-box; max-width: 40px; }
             p { max-width: none; }",
        )];
        let s = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(s.min_width, Size::Auto);
        assert_eq!(s.max_height, MaxSize::None);
        assert_eq!(s.box_sizing, BoxSizing::ContentBox);
        assert_eq!(s.max_width, MaxSize::None);
        let sheets = [sheet("div { max-width: 100px; } p { max-width: inherit; }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").max_width,
            MaxSize::Px(100.0)
        );
    }

    #[test]
    fn white_space_is_the_shorthand_of_collapse_and_wrap_mode() {
        let ws = |css: &str| {
            let sheets = [sheet(&format!("p {{ {css} }}"))];
            style_of("<p>x</p>", &sheets, "p").white_space
        };
        let pair = |collapse, wrap| WhiteSpace { collapse, wrap };
        for (value, expected) in [
            ("normal", WhiteSpace::NORMAL),
            ("pre", WhiteSpace::PRE),
            ("pre-wrap", WhiteSpace::PRE_WRAP),
            ("pre-line", WhiteSpace::PRE_LINE),
            ("nowrap", WhiteSpace::NOWRAP),
            ("break-spaces", WhiteSpace::BREAK_SPACES),
            (
                "preserve-spaces",
                pair(WhiteSpaceCollapse::PreserveSpaces, true),
            ),
            (
                "nowrap preserve-breaks",
                pair(WhiteSpaceCollapse::PreserveBreaks, false),
            ),
            ("wrap", WhiteSpace::NORMAL),
        ] {
            assert_eq!(ws(&format!("white-space: {value}")), expected, "{value}");
        }
        for invalid in ["wrap wrap", "pre nowrap", "collapse preserve", "bogus"] {
            assert_eq!(
                ws(&format!("white-space: pre; white-space: {invalid}")),
                WhiteSpace::PRE,
                "{invalid}"
            );
        }
        assert_eq!(
            ws("white-space: pre; text-wrap-mode: wrap"),
            WhiteSpace::PRE_WRAP
        );
        assert_eq!(
            ws("white-space-collapse: preserve-breaks"),
            WhiteSpace::PRE_LINE
        );
        assert_eq!(
            ws("text-wrap: balance; white-space-collapse: preserve"),
            WhiteSpace::PRE_WRAP
        );
        assert_eq!(ws("text-wrap: nowrap"), WhiteSpace::NOWRAP);
        assert_eq!(
            ws("white-space: pre; text-wrap: balance"),
            WhiteSpace::PRE_WRAP
        );
        // Inherited, longhand by longhand.
        let sheets = [sheet("div { white-space: pre } p { text-wrap-mode: wrap }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").white_space,
            WhiteSpace::PRE_WRAP
        );
    }

    #[test]
    fn text_breaking_visibility_and_text_shadow_are_computed_and_inherited() {
        use crate::computed::{OverflowWrap, Visibility, WordBreak};
        let sheets = [sheet(
            "div { overflow-wrap: anywhere; word-break: keep-all; visibility: hidden; \
             text-shadow: 1px 2px red, blue 0 0 3px } \
             i { word-wrap: break-word; word-break: break-all; visibility: visible } \
             b { visibility: collapse; text-shadow: none } \
             u { text-shadow: 1px 2px 3px 4px } s { text-shadow: inset 1px 1px }",
        )];
        let html = "<div><p>x</p><i>y</i><b>z</b><u>u</u><s>s</s></div>";
        let p = style_of(html, &sheets, "p");
        assert_eq!(
            (p.overflow_wrap, p.word_break, p.visibility),
            (
                OverflowWrap::Anywhere,
                WordBreak::KeepAll,
                Visibility::Hidden
            )
        );
        assert_eq!(p.decoration().text_shadow.len(), 2);
        assert_eq!(p.decoration().text_shadow[1].blur, 3.0);
        let i = style_of(html, &sheets, "i");
        assert_eq!(
            (i.overflow_wrap, i.word_break, i.visibility),
            (
                OverflowWrap::BreakWord,
                WordBreak::BreakAll,
                Visibility::Visible
            )
        );
        let unset = [sheet("div { visibility: hidden } p { visibility: unset }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &unset, "p").visibility,
            Visibility::Hidden
        );
        let b = style_of(html, &sheets, "b");
        assert_eq!(b.visibility, Visibility::Collapse);
        assert!(b.decoration().text_shadow.is_empty());
        // A spread or `inset` is no text shadow: the inherited one stands.
        for tag in ["u", "s"] {
            assert_eq!(
                style_of(html, &sheets, tag).decoration().text_shadow.len(),
                2,
                "{tag}"
            );
        }
        let value = |name| crate::serialize::computed_value(&p, name).unwrap();
        assert_eq!(
            value("text-shadow"),
            "rgb(255, 0, 0) 1px 2px 0px, rgb(0, 0, 255) 0px 0px 3px"
        );
        assert_eq!(value("overflow-wrap"), "anywhere");
        assert_eq!(value("word-break"), "keep-all");
        assert_eq!(value("visibility"), "hidden");
        let pre_line = style_of(
            "<p>x</p>",
            &[sheet("p { white-space: preserve-breaks nowrap }")],
            "p",
        );
        let value = |name| crate::serialize::computed_value(&pre_line, name).unwrap();
        assert_eq!(value("white-space"), "preserve-breaks nowrap");
        assert_eq!(value("white-space-collapse"), "preserve-breaks");
        assert_eq!(value("text-wrap-mode"), "nowrap");
    }

    #[test]
    fn letter_and_word_spacing_accept_lengths_and_normal() {
        let sheets = [sheet(
            "p { letter-spacing: 2px; word-spacing: 1em; font-size: 10px; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.letter_spacing, 2.0);
        assert_eq!(s.word_spacing, 10.0); // 1em at font-size: 10px

        let sheets = [sheet(
            "div { letter-spacing: 5px; } p { letter-spacing: normal; }",
        )];
        let s = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(
            s.letter_spacing, 0.0,
            "`normal` resets, it doesn't just fail to parse"
        );
    }

    #[test]
    fn dir_attribute_sets_direction_and_inherits() {
        let s = style_of(r#"<div dir="rtl"><span>x</span></div>"#, &[], "span");
        assert_eq!(
            s.direction,
            Direction::Rtl,
            "inherited from the div's dir attribute"
        );

        let s = style_of(r#"<p dir="ltr">x</p>"#, &[], "p");
        assert_eq!(s.direction, Direction::Ltr);

        // Unrecognized (e.g. `auto`, which v1 doesn't content-sniff for)
        // leaves the inherited/initial value alone rather than erroring.
        let s = style_of(r#"<p dir="auto">x</p>"#, &[], "p");
        assert_eq!(s.direction, Direction::Ltr);
    }

    #[test]
    fn width_accepts_auto_px_and_percent() {
        let auto = style_of("<p>x</p>", &[], "p");
        assert_eq!(auto.width, Size::Auto); // initial value

        let sheets = [sheet("p { width: 200px; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").width, Size::Px(200.0));

        let sheets = [sheet("p { width: 50%; }")];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "p").width,
            Size::Percent(50.0)
        );
    }

    #[test]
    fn height_margins_and_padding_keep_percentages_for_layout() {
        let sheets = [sheet("p { height: 300px; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").height, Size::Px(300.0));
        // A percentage is kept as written (M10.1): only layout knows the
        // containing block it is a percentage of.
        let sheets = [sheet(
            "p { height: 50%; margin: 5% auto; padding: 10% 2px; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.height, Size::Percent(50.0));
        assert_eq!(
            s.margin,
            [
                LengthPercentageAuto::Percent(5.0),
                LengthPercentageAuto::Auto,
                LengthPercentageAuto::Percent(5.0),
                LengthPercentageAuto::Auto,
            ]
        );
        assert_eq!(
            s.padding,
            [
                LengthPercentage::Percent(10.0),
                LengthPercentage::Px(2.0),
                LengthPercentage::Percent(10.0),
                LengthPercentage::Px(2.0),
            ]
        );
        // Padding takes no negative percentage; a margin may be negative.
        let sheets = [sheet("p { padding: -5%; margin: -5%; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.padding, [const { LengthPercentage::Px(0.0) }; 4]);
        assert_eq!(s.margin[TOP], LengthPercentageAuto::Percent(-5.0));
    }

    /// A font that answers `ex` with 0.4em, `ch` with 0.6em, `cap` with
    /// 0.7em, `ic` with 0.9em and a normal line with 1.25em, so a test
    /// can tell the trait's answer from the fallbacks; a family named
    /// `wide` twice each.
    struct StubFont;

    impl FontMeasure for StubFont {
        fn units(&self, font: &FontProps, size: f32) -> FontUnits {
            let k = if font.family.contains("wide") {
                2.0
            } else {
                1.0
            };
            // A fixed-width family's zero is narrower.
            let zero = if &*font.family == "monospace" {
                0.5
            } else {
                0.6
            };
            FontUnits {
                x_height: size * 0.4 * k,
                zero_advance: size * zero * k,
                cap_height: size * 0.7 * k,
                ic_advance: size * 0.9 * k,
                normal_line_height: size * 1.25 * k,
            }
        }
    }

    /// A context with [`StubFont`] in an 800 by 600 viewport.
    fn stub_context() -> CascadeContext<'static> {
        CascadeContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            fonts: Some(&StubFont),
            language: None,
            media: Media::screen(800.0, 600.0),
            animations: None,
            inline_styles: None,
            frame_margins: FrameMargins::default(),
            top_layer: None,
        }
    }

    #[test]
    fn the_font_relative_units_measure_the_elements_font_and_line() {
        let context = stub_context();
        let sheets = [sheet(
            "html { font-size: 20px; line-height: 30px; font-family: wide }
             body { font-family: serif; line-height: 2lh }
             p { width: 1ch; font-size: 10px; line-height: 2; font-family: wide;
                 margin: 1cap 1ic 1lh 1rlh; padding: 1rex 1rch 1rcap 1ric }
             span { font-size: 2ex; font-weight: calc(1em / 1px) }",
        )];
        let html = "<p>x<span>y</span></p>";
        let p = style_of_in(html, &sheets, "p", &context);
        use LengthPercentageAuto::Px;
        assert_eq!(p.width, Size::Px(12.0), "ch of its own wide font, after it");
        assert_eq!(
            p.margin,
            [Px(14.0), Px(18.0), Px(20.0), Px(30.0)],
            "cap, ic and lh its own, rlh the root's line"
        );
        assert_eq!(
            p.padding,
            [16.0, 24.0, 28.0, 36.0].map(LengthPercentage::Px),
            "the root's font at the root's size"
        );
        // `lh` in `line-height` is the parent's line.
        let body = style_of_in(html, &sheets, "body", &context);
        assert_eq!(
            body.font().line_height,
            crate::computed::LineHeight::Px(60.0)
        );
        // In the font properties the units are the parent's.
        let span = style_of_in(html, &sheets, "span", &context);
        assert_eq!(span.font_size_px, 16.0, "2ex of the parent's wide 10px");
        assert_eq!(span.font().weight, 10.0, "em of the parent's 10px");
        // On the root the root units are the initial font's in its font
        // properties.
        let sheets = [sheet(
            "html { font-size: 20px; line-height: 2rlh; margin: 1rlh }",
        )];
        let root = style_of_in(html, &sheets, "html", &context);
        assert_eq!(
            root.font().line_height,
            crate::computed::LineHeight::Px(40.0),
            "2 of 1.25 * 16"
        );
        assert_eq!(root.margin[TOP], Px(40.0), "its own line elsewhere");
    }

    #[test]
    fn a_color_mix_of_currentcolor_is_inherited_unresolved() {
        let sheets = [sheet(
            "div { color: red; background-color: color-mix(in srgb, currentColor 50%, lime) }
             p { color: blue; background-color: inherit }
             span { color: lime }",
        )];
        let html = "<div><p>x<span>y</span></p></div>";
        let channels = |c: Color| [c.r, c.g, c.b].map(|v| (v * 100.0).round() / 100.0);
        assert_eq!(
            channels(style_of(html, &sheets, "div").background_color),
            [0.5, 0.5, 0.0]
        );
        // The child mixes its own colour, not its parent's mix.
        assert_eq!(
            channels(style_of(html, &sheets, "p").background_color),
            [0.0, 0.5, 0.5]
        );
        // An element that sets none has none.
        assert_eq!(style_of(html, &sheets, "span").background().color_mix, None);
        // A relative colour from it and a contrast with it likewise
        // (D78).
        for (value, own, child) in [
            (
                "rgb(from currentcolor calc(255 - r) g b)",
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 1.0],
            ),
            ("contrast-color(currentcolor)", [1.0; 3], [0.0; 3]),
        ] {
            let sheets = [sheet(&format!(
                "div {{ color: black; background-color: {value} }}
                 p {{ color: white; background-color: inherit }}"
            ))];
            let html = "<div><p>x</p></div>";
            let of = |selector| channels(style_of(html, &sheets, selector).background_color);
            assert_eq!(of("div"), own, "{value}");
            assert_eq!(of("p"), child, "{value}: the child's own colour");
        }
    }

    #[test]
    fn font_weight_and_style_take_em_of_the_parent() {
        let sheets = [sheet(
            "div { font-size: 10px } p { font-size: 100px; font-weight: calc(1em / 1px);
             font-style: oblique calc(1em / 1px * 1deg) }",
        )];
        let p = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(p.font().weight, 10.0);
        assert_eq!(p.font().style, crate::computed::FontStyle::Oblique(10.0));
    }

    /// [`style_of`] with a cascade context.
    fn style_of_in(
        html: &str,
        sheets: &[css::Stylesheet],
        selector: &str,
        context: &CascadeContext<'_>,
    ) -> ComputedStyle {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let styles = cascade_document(&document, sheets, context);
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        document
            .subtree(document.root())
            .find(|&id| matches_selector_list(&document, id, &list))
            .and_then(|id| styles.get(id).cloned())
            .expect("the selector matches an element")
    }

    #[test]
    fn relative_units_resolve_against_the_context() {
        let context = stub_context();
        let sheets = [sheet(
            "html { font-size: 20px }
             p { font-size: 10px; margin: 2rem 1ex 1ch 1em; padding: 10vw 10vh 10vmin 10vmax;
                 width: 1in; letter-spacing: 12pt; }",
        )];
        let s = style_of_in("<p>x</p>", &sheets, "p", &context);
        use LengthPercentageAuto::Px;
        assert_eq!(
            s.margin,
            [Px(40.0), Px(4.0), Px(6.0), Px(10.0)],
            "rem of the root's 20px, ex and ch from the font, em of its own 10px"
        );
        assert_eq!(
            s.padding,
            [
                LengthPercentage::Px(80.0),
                LengthPercentage::Px(60.0),
                LengthPercentage::Px(60.0),
                LengthPercentage::Px(80.0),
            ],
            "vw, vh, vmin, vmax of 800 by 600"
        );
        assert_eq!(s.width, Size::Px(96.0), "an inch is 96px");
        assert_eq!(s.letter_spacing, 16.0, "12pt is 16px");
        for (value, px) in [
            ("2.54cm", 96.0),
            ("25.4mm", 96.0),
            ("40q", 96.0 / 2.54),
            ("1pc", 16.0),
        ] {
            let sheets = [sheet(&format!("p {{ width: {value} }}"))];
            let w = style_of_in("<p>x</p>", &sheets, "p", &context).width;
            let Size::Px(got) = w else {
                panic!("{value} is a length");
            };
            assert!((got - px).abs() < 1e-3, "{value}: {got} != {px}");
        }
    }

    #[test]
    fn ch_is_the_zero_of_the_elements_own_font_family() {
        let context = CascadeContext {
            fonts: Some(&StubFont),
            ..CascadeContext::default()
        };
        let sheets = [sheet(
            "p { font-size: 10px; width: 10ch }              .mono { font-family: monospace } span { width: 10ch } \
             .short { font: 10px monospace }",
        )];
        let html = "<p>x</p><p class=mono><span>y</span></p><p class=short>z</p>";
        let px = |selector| match style_of_in(html, &sheets, selector, &context).width {
            Size::Px(px) => px,
            other => panic!("{other:?}"),
        };
        assert_eq!(px("p"), 60.0, "the primary font's zero");
        assert_eq!(px(".mono"), 50.0, "the family the element declares");
        assert_eq!(px("span"), 50.0, "the family it inherits");
        assert_eq!(px(".short"), 50.0, "the family the shorthand sets");
    }

    #[test]
    fn rem_is_the_root_font_size_and_the_initial_value_on_the_root() {
        let sheets = [sheet(
            "html { font-size: 2rem } body { margin: 1rem } p { font-size: 150% }",
        )];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "html").font_size_px,
            32.0,
            "on the root, rem is the initial 16px"
        );
        assert_eq!(
            style_of("<p>x</p>", &sheets, "body").margin[TOP],
            LengthPercentageAuto::Px(32.0)
        );
        assert_eq!(
            style_of("<p>x</p>", &sheets, "p").font_size_px,
            48.0,
            "a percentage font size is of the parent's, the body's, which \
             it inherits from the root"
        );
        // The root's other properties take its own font size.
        let sheets = [sheet(":root { font-size: 50px; margin-left: 2rem }")];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "html").margin[LEFT],
            LengthPercentageAuto::Px(100.0)
        );
    }

    #[test]
    fn viewport_and_font_units_fall_back_without_a_context_and_say_when_used() {
        // No context: the viewport is zero and the font units are half
        // an em (css-values-4 §6.1.1's fallbacks).
        let sheets = [sheet("p { width: 10vw; margin: 1ex 1ch 0 0 }")];
        let document = Document::from_node(&html::parse_document("<p>x</p>").expect("parses"));
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        assert!(styles.uses_viewport_units);
        let p = styles.get(first(&document, "p")).expect("styled");
        assert_eq!(p.width, Size::Px(0.0));
        assert_eq!(p.margin[TOP], LengthPercentageAuto::Px(8.0));
        assert_eq!(p.margin[RIGHT], LengthPercentageAuto::Px(8.0));
        // A document without viewport units says so, so a resize need
        // not recascade it.
        let sheets = [sheet("p { width: 10em; margin: 1rem }")];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        assert!(!styles.uses_viewport_units);
        // The units of a font or a line are said to be used, em and rem not.
        let context = stub_context();
        let styles = cascade_document(&document, &sheets, &context);
        assert!(!styles.uses_font_units);
        let sheets = [sheet("p { width: 1cap }")];
        assert!(cascade_document(&document, &sheets, &context).uses_font_units);
        // A unit nobody knows is not a length; an infinite one is
        // rejected even through a viewport unit.
        let sheets = [sheet("p { width: 10furlong; margin-top: 1e39vw }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.width, Size::Auto);
        assert_eq!(
            s.margin[TOP],
            LengthPercentageAuto::Px(16.0),
            "the UA sheet's margin stands"
        );
    }

    #[test]
    fn border_shorthand_sets_width_style_and_color_in_any_order() {
        let sheets = [sheet("p { border: 2px solid red; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [2.0; 4]);
        assert_eq!(s.border_style, [BorderStyle::Solid; 4]);
        assert_eq!(s.border_color, [Color::rgb8(255, 0, 0); 4]);

        // Order-independence.
        let sheets = [sheet("p { border: red solid 2px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [2.0; 4]);
        assert_eq!(s.border_color, [Color::rgb8(255, 0, 0); 4]);
        // An omitted width is `medium`.
        let sheets = [sheet("p { border: solid; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").border_width, [3.0; 4]);
    }

    #[test]
    fn aspect_ratio_and_one_axis_of_a_background_position_are_computed_values() {
        // Gap batch E (G36, G40): the reference browser's computed values.
        let sheets = [sheet(
            "p { aspect-ratio: 16 / 9; background-position: 10px 20%, right }              i { aspect-ratio: auto 2 / 1 }",
        )];
        let html = "<p>x<i>y</i><b>z</b></p>";
        let value = |selector: &str, name: &str| {
            crate::serialize::computed_value(&style_of(html, &sheets, selector), name)
        };
        assert_eq!(value("p", "aspect-ratio").as_deref(), Some("16 / 9"));
        assert_eq!(value("i", "aspect-ratio").as_deref(), Some("auto 2 / 1"));
        assert_eq!(value("b", "aspect-ratio").as_deref(), Some("auto"));
        assert_eq!(
            value("p", "background-position-x").as_deref(),
            Some("10px, 100%")
        );
        assert_eq!(
            value("p", "background-position-y").as_deref(),
            Some("20%, 50%")
        );
        assert_eq!(value("b", "background-position-x").as_deref(), Some("0%"));
        // A position's percentages wait for their basis, which may be
        // negative, before they are compared.
        let sheets = [sheet("p { background-position: min(0%, 100%) 0 }")];
        let p = style_of(html, &sheets, "p");
        assert!(matches!(
            p.background().position_x[0],
            LengthPercentage::Calc(_)
        ));
    }

    #[test]
    fn currentcolor_keeps_its_keyword_and_is_inherited_as_it() {
        // D66, G226: the keyword is inherited, explicitly or by an
        // inherited property, and is the inheriting element's colour.
        let html = "<div style=\"color: red; border-color: currentcolor; \
                    background-color: currentcolor; text-shadow: 1px 1px; \
                    box-shadow: currentcolor 1px 1px; \
                    background-image: linear-gradient(currentcolor, red)\">\
                    <p style=\"color: blue; border-color: inherit; \
                    background-color: inherit; box-shadow: inherit; \
                    background-image: inherit\">x</p></div>";
        let blue = Color::rgb8(0, 0, 255);
        let p = style_of(html, &[], "p");
        assert_eq!(p.border_color.map(Color::used), [blue; 4]);
        assert!(p.border_color_is_current(TOP));
        assert_eq!(p.background_color.used(), blue);
        assert_eq!(p.decoration().text_shadow[0].color.used(), blue);
        assert_eq!(p.effects().box_shadow[0].color.used(), blue);
        let Some(Some(crate::computed::Image::Gradient(g))) = p.background().image.first() else {
            panic!("a gradient");
        };
        assert_eq!(
            g.items[0],
            crate::computed::GradientItem::Stop(Color::CURRENT.resolve(blue), None)
        );
        let div = style_of(html, &[], "div");
        assert_eq!(div.border_color[TOP].used(), Color::rgb8(255, 0, 0));
        // A colour mixed from it is the colour it is where it computes
        // (section 2 of the note).
        let html = "<div style=\"color: red; \
                    border-color: color-mix(in srgb, currentcolor, red)\">\
                    <p style=\"color: blue; border-color: inherit\">x</p></div>";
        let p = style_of(html, &[], "p");
        let top = p.border_color[TOP];
        assert_eq!((top.r, top.g, top.b), (1.0, 0.0, 0.0));
        assert!(!p.border_color_is_current(TOP));
    }

    #[test]
    fn currentcolor_is_the_elements_colour_in_every_colour_property() {
        // Gap batch E: the reference browser's computed values.
        let sheets = [sheet(
            "div { color: rgb(0, 128, 0) } \
             p { background-color: currentcolor; \
                 border: 1px solid color-mix(in srgb, currentcolor, blue); \
                 outline-color: color-mix(in srgb, currentColor 50%, red); \
                 text-decoration-color: color-mix(in srgb, currentcolor, white) } \
             span { color: currentcolor; background: linear-gradient(currentcolor, red) } \
             b { color: color-mix(in srgb, currentcolor, blue) } \
             i { color: blue; background-color: currentcolor }",
        )];
        let html = "<div><p>x<span>y</span><b>z</b><i>w</i></p></div>";
        // The element's own colour, not the one it inherits.
        let i = style_of(html, &sheets, "i");
        assert_eq!(
            i.background_color,
            Color::CURRENT.resolve(Color::rgb8(0, 0, 255))
        );
        let p = style_of(html, &sheets, "p");
        let value = |s: &ComputedStyle, name: &str| crate::serialize::computed_value(s, name);
        assert_eq!(
            value(&p, "background-color").as_deref(),
            Some("rgb(0, 128, 0)")
        );
        assert_eq!(
            value(&p, "border-top-color").as_deref(),
            Some("color(srgb 0 0.25098 0.5)")
        );
        assert_eq!(
            value(&p, "outline-color").as_deref(),
            Some("color(srgb 0.5 0.25098 0)")
        );
        let span = style_of(html, &sheets, "span");
        assert_eq!(span.color, Color::rgb8(0, 128, 0));
        // The parent's colour is `color`'s own `currentcolor`.
        let b = style_of(html, &sheets, "b");
        assert_eq!(
            value(&b, "color").as_deref(),
            Some("color(srgb 0 0.25098 0.5)")
        );
        // A colour of another space computes as itself.
        let sheets = [sheet("p { color: oklch(60% 0.2 400deg / 50%) }")];
        let p = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(
            value(&p, "color").as_deref(),
            Some("oklch(0.6 0.2 40 / 0.5)")
        );
    }

    /// The styles of `html` under `css`, the user preferring `scheme`.
    fn schemed(html: &str, css: &str, scheme: crate::ColorScheme) -> (Document, Styles) {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let mut media = Media::screen(800.0, 600.0);
        media.preferences.color_scheme = scheme;
        let context = CascadeContext {
            media,
            ..CascadeContext::default()
        };
        let styles = cascade_document(&document, &[sheet(css)], &context);
        (document, styles)
    }

    #[test]
    fn the_colour_scheme_decides_system_colours_light_dark_and_the_roots_colour() {
        use crate::ColorScheme::{Dark, Light};
        let value = |styles: &Styles, document: &Document, tag: &str, name: &str| {
            crate::serialize::computed_value(
                styles.get(first(document, tag)).expect("styled"),
                name,
            )
            .unwrap()
        };
        // Gap batch E, D49: the reference browser's values.
        let css = "#d { color-scheme: dark } \
                   p { background-color: Canvas; border-top-color: light-dark(red, blue); \
                       outline-color: LinkText }";
        let html = "<div id=d><p>x</p></div><span><p>y</p></span>";
        let (document, styles) = schemed(html, css, Light);
        let p = styles.get(first(&document, "p")).unwrap();
        assert!(p.dark);
        assert_eq!(p.background_color, Color::rgb8(18, 18, 18));
        assert_eq!(p.border_color[TOP], Color::rgb8(0, 0, 255));
        assert_eq!(p.outline_color(), Color::rgb8(158, 158, 255));
        // An element's scheme leaves the colour it inherits.
        assert_eq!(p.color, Color::BLACK);
        assert_eq!(value(&styles, &document, "div", "color-scheme"), "dark");
        assert_eq!(value(&styles, &document, "span", "color-scheme"), "normal");
        // Words as written, `only` last; what is not a value is dropped.
        for (written, computed) in [
            ("light dark", "light dark"),
            ("only dark", "dark only"),
            ("dark light foo", "dark light foo"),
            ("normal", "normal"),
            ("normal dark", "normal"),
            ("only", "normal"),
            ("dark only only", "normal"),
            // A CSS-wide keyword is no scheme, and `only` is at an end.
            ("light inherit", "normal"),
            ("light revert-layer", "normal"),
            ("light only dark", "normal"),
            ("only light", "light only"),
        ] {
            let (document, styles) = schemed(
                "<div>x</div>",
                &format!("div {{ color-scheme: {written} }}"),
                Light,
            );
            assert_eq!(
                value(&styles, &document, "div", "color-scheme"),
                computed,
                "{written}"
            );
        }
        // The root's: its `color` is `CanvasText` by it.
        let (document, styles) = schemed("<p>x</p>", ":root { color-scheme: dark }", Light);
        assert_eq!(
            value(&styles, &document, "p", "color"),
            "rgb(255, 255, 255)"
        );
        // `light dark` is the user's choice.
        let css = ":root { color-scheme: light dark } p { background: light-dark(red, blue) }";
        let (document, styles) = schemed("<p>x</p>", css, Light);
        assert_eq!(
            value(&styles, &document, "p", "background-color"),
            "rgb(255, 0, 0)"
        );
        assert_eq!(value(&styles, &document, "p", "color"), "rgb(0, 0, 0)");
        let (document, styles) = schemed("<p>x</p>", css, Dark);
        assert_eq!(
            value(&styles, &document, "p", "background-color"),
            "rgb(0, 0, 255)"
        );
        assert_eq!(
            value(&styles, &document, "p", "color"),
            "rgb(255, 255, 255)"
        );
        // `normal` is the page's, its meta element's; the property wins.
        let html = "<head><meta name=color-scheme content='dark'></head><p>x</p>";
        let (document, styles) = schemed(html, "", Light);
        assert_eq!(
            value(&styles, &document, "p", "color"),
            "rgb(255, 255, 255)"
        );
        let (document, styles) = schemed(html, ":root { color-scheme: light }", Light);
        assert_eq!(value(&styles, &document, "p", "color"), "rgb(0, 0, 0)");
        let html = "<head><meta name=color-scheme content='light dark'></head><p>x</p>";
        let (document, styles) = schemed(html, "", Light);
        assert_eq!(value(&styles, &document, "p", "color"), "rgb(0, 0, 0)");
        // `color: initial` is `CanvasText` by the element's scheme.
        let (document, styles) = schemed(
            "<p>x</p>",
            "p { color-scheme: dark; color: initial }",
            Light,
        );
        assert_eq!(
            value(&styles, &document, "p", "color"),
            "rgb(255, 255, 255)"
        );
    }

    #[test]
    fn border_radius_computes_as_the_reference_browser_writes_it() {
        // Gap batch E, D50.
        let value = |css: &str, name: &str| {
            let sheets = [sheet(&format!("div {{ {css} }}"))];
            let style = style_of("<div>x</div>", &sheets, "div");
            crate::serialize::computed_value(&style, name).unwrap()
        };
        assert_eq!(
            value("border-radius: 10px", "border-top-left-radius"),
            "10px"
        );
        let css = "border-radius: 10px 20px / 5px";
        assert_eq!(value(css, "border-top-left-radius"), "10px 5px");
        assert_eq!(value(css, "border-top-right-radius"), "20px 5px");
        assert_eq!(value(css, "border-bottom-right-radius"), "10px 5px");
        assert_eq!(value(css, "border-bottom-left-radius"), "20px 5px");
        let css = "border-radius: 1px 2px 3px";
        assert_eq!(value(css, "border-bottom-left-radius"), "2px");
        assert_eq!(value(css, "border-bottom-right-radius"), "3px");
        assert_eq!(
            value("border-top-left-radius: 10px 20%", "border-top-left-radius"),
            "10px 20%"
        );
        // A negative radius, or a third value, is no value.
        assert_eq!(
            value(
                "border-radius: 4px; border-radius: -1px",
                "border-top-left-radius"
            ),
            "4px"
        );
        assert_eq!(
            value(
                "border-radius: 4px; border-radius: 1px / 2px / 3px",
                "border-top-left-radius"
            ),
            "4px"
        );
        assert_eq!(
            value(
                "border-top-left-radius: 4px; border-top-left-radius: 1px 2px 3px",
                "border-top-left-radius"
            ),
            "4px"
        );
        // Radii set back to zero keep no copy.
        let sheets = [sheet("div { border-radius: 3px; border-radius: 0 }")];
        assert!(style_of("<div>x</div>", &sheets, "div").radii.is_none());
    }

    #[test]
    fn filter_computes_as_the_reference_browser_writes_it() {
        // Gap batch E, D52: the reference browser's computed values.
        let value = |css: &str| {
            let sheets = [sheet(&format!("div {{ {css} }}"))];
            let style = style_of("<div>x</div>", &sheets, "div");
            crate::serialize::computed_value(&style, "filter").unwrap()
        };
        assert_eq!(
            value("filter: blur(2px) brightness(0.5)"),
            "blur(2px) brightness(0.5)"
        );
        assert_eq!(value("filter: grayscale()"), "grayscale(1)");
        assert_eq!(
            value("filter: drop-shadow(1px 2px 3px red)"),
            "drop-shadow(rgb(255, 0, 0) 1px 2px 3px)"
        );
        assert_eq!(
            value(
                "filter: hue-rotate(90deg) saturate(200%) sepia(1) invert(50%) opacity(0.5) contrast(2)"
            ),
            "hue-rotate(90deg) saturate(2) sepia(1) invert(0.5) opacity(0.5) contrast(2)"
        );
        assert_eq!(
            value("filter: drop-shadow(red 1px 2px)"),
            "drop-shadow(rgb(255, 0, 0) 1px 2px 0px)"
        );
        assert_eq!(
            value("filter: grayscale(250%) blur()"),
            "grayscale(1) blur(0px)"
        );
        assert_eq!(value("filter: hue-rotate(0.5turn)"), "hue-rotate(180deg)");
        // A calculation below the range is clamped to it.
        assert_eq!(
            value("filter: brightness(calc(-10)) contrast(calc(-10%))"),
            "brightness(0) contrast(0)"
        );
        for bad in [
            "blur(-1px)",
            "url(#f)",
            "brightness(-1)",
            "drop-shadow(1px 2px 3px 4px red)",
            "drop-shadow(inset 1px 2px)",
            "blur(1px) nope(1)",
            "blur(1px), opacity(1)",
        ] {
            assert_eq!(
                value(&format!("filter: invert(1); filter: {bad}")),
                "invert(1)",
                "{bad}"
            );
        }
        // A filter makes a stacking context and a containing block.
        let sheets = [sheet("div { filter: blur(1px) }")];
        let style = style_of("<div>x</div>", &sheets, "div");
        assert!(style.effects().is_stacking_context());
        assert!(style.effects().contains_all_of(true));
    }

    #[test]
    fn box_shadow_computes_as_the_reference_browser_writes_it() {
        // Gap batch E, D52: the reference browser's computed values.
        let value = |css: &str| {
            let sheets = [sheet(&format!("div {{ color: rgb(0, 128, 0); {css} }}"))];
            let style = style_of("<div>x</div>", &sheets, "div");
            crate::serialize::computed_value(&style, "box-shadow").unwrap()
        };
        assert_eq!(
            value("box-shadow: 1px 2px 3px 4px red"),
            "rgb(255, 0, 0) 1px 2px 3px 4px"
        );
        assert_eq!(
            value("box-shadow: inset 0 0 5px blue, 1px 1px"),
            "rgb(0, 0, 255) 0px 0px 5px 0px inset, rgb(0, 128, 0) 1px 1px 0px 0px"
        );
        assert_eq!(
            value("box-shadow: 1px 2px red inset"),
            "rgb(255, 0, 0) 1px 2px 0px 0px inset"
        );
        assert_eq!(
            value("box-shadow: currentcolor 1px 2px"),
            "rgb(0, 128, 0) 1px 2px 0px 0px"
        );
        assert_eq!(value("box-shadow: none"), "none");
        // A calculated blur below zero is clamped to it.
        assert_eq!(
            value("box-shadow: 1px 1px calc(-1px)"),
            "rgb(0, 128, 0) 1px 1px 0px 0px"
        );
        for bad in [
            "1px 2px -3px red",
            "1px",
            "1px 2px 3px 4px 5px",
            "1px red 2px",
            "inset 1px 2px inset",
            "red 1px 2px blue",
            "1px 2px,",
        ] {
            assert_eq!(
                value(&format!("box-shadow: 4px 4px; box-shadow: {bad}")),
                "rgb(0, 128, 0) 4px 4px 0px 0px",
                "{bad}"
            );
        }
        let many = vec!["1px 1px"; MAX_SHADOWS + 1].join(", ");
        assert_eq!(value(&format!("box-shadow: {many}")), "none");
    }

    #[test]
    fn flow_root_computes_as_the_reference_browser_writes_it() {
        // Gap batch E.
        for (written, computed) in [
            ("flow-root", "flow-root"),
            ("block flow-root", "flow-root"),
            ("inline flow-root", "inline-block"),
            ("flow-root list-item", "flow-root list-item"),
            ("flow-root; display: block", "block"),
            ("flow flow-root list-item", "block"),
        ] {
            let sheets = [sheet(&format!("div {{ display: {written} }}"))];
            let style = style_of("<div>x</div>", &sheets, "div");
            assert_eq!(
                crate::serialize::computed_value(&style, "display").as_deref(),
                Some(computed),
                "{written}"
            );
        }
        // The root's box is a formatting context root, not a flow root.
        let root = style_of("<p>x</p>", &[], "html");
        assert!(root.root && !root.flow_root);
        assert!(!style_of("<p>x</p>", &[], "p").root);
    }

    #[test]
    fn a_change_of_the_users_scheme_is_a_change_of_media() {
        // D49: `color-scheme` reads it with no `@media` rule to.
        let document = Document::from_node(&html::parse_document("<p>x</p>").expect("parses"));
        let sheets = Stylesheets::new(vec![sheet(":root { color-scheme: light dark }")]);
        let light = Media::screen(800.0, 600.0);
        let mut dark = light;
        dark.preferences.color_scheme = crate::ColorScheme::Dark;
        let context = CascadeContext {
            media: light,
            ..CascadeContext::default()
        };
        let styles = sheets.cascade(&document, &context);
        assert!(!sheets.media_changed(&styles, &light));
        assert!(sheets.media_changed(&styles, &dark));
        // A page whose schemes the preference chooses none of is left be.
        let sheets = Stylesheets::new(vec![sheet(":root { color-scheme: dark }")]);
        let styles = sheets.cascade(&document, &context);
        assert!(!sheets.media_changed(&styles, &dark));
    }

    #[test]
    fn dark_controls_and_links_take_the_reference_browsers_dark_colours() {
        let html = "<div style='color-scheme: dark'><input id=i><button id=b>b</button>\
                    <input id=dis disabled><button id=bd disabled>x</button><a href=x>a</a></div>\
                    <input id=li><button id=lb>b</button>";
        let (document, styles) = schemed(html, "", crate::ColorScheme::Light);
        let of = |id: &str| {
            let node = document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some(id))
                .unwrap_or_else(|| panic!("#{id}"));
            let style = styles.get(node).expect("styled");
            ["color", "background-color", "border-top-color"]
                .map(|name| crate::serialize::computed_value(style, name).unwrap())
                .join("; ")
        };
        assert_eq!(
            of("i"),
            "rgb(255, 255, 255); rgb(59, 59, 59); rgb(133, 133, 133)"
        );
        assert_eq!(
            of("b"),
            "rgb(255, 255, 255); rgb(107, 107, 107); rgb(255, 255, 255)"
        );
        assert_eq!(
            of("dis"),
            "rgb(170, 170, 170); rgba(59, 59, 59, 0.3); rgba(118, 118, 118, 0.3)"
        );
        assert_eq!(
            of("bd"),
            "rgba(255, 255, 255, 0.3); rgba(19, 1, 1, 0.3); rgba(195, 195, 195, 0.3)"
        );
        // The light ones are what they were (M26.7).
        assert_eq!(
            of("li"),
            "rgb(0, 0, 0); rgb(255, 255, 255); rgb(118, 118, 118)"
        );
        assert_eq!(
            of("lb"),
            "rgb(0, 0, 0); rgb(240, 240, 240); rgb(118, 118, 118)"
        );
        let a = styles.get(first(&document, "a")).unwrap();
        assert_eq!(a.color, Color::rgb8(158, 158, 255));
    }

    #[test]
    fn border_color_defaults_to_currentcolor() {
        let sheets = [sheet("p { color: red; border: 1px solid; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        let red = Color::CURRENT.resolve(Color::rgb8(255, 0, 0));
        assert_eq!(s.border_color, [red; 4]);
        // Per side: a side with its own color keeps it, the others follow `color`.
        let sheets = [sheet(
            "p { color: red; border: 1px solid; border-left-color: blue; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_color[LEFT], Color::rgb8(0, 0, 255));
        assert_eq!(s.border_color[TOP], red);
        let current = |s: &ComputedStyle| {
            (0..4)
                .map(|side| s.border_color_is_current(side))
                .collect::<Vec<_>>()
        };
        assert_eq!(current(&s), [true, true, true, false]);
        // The keyword itself (M33), in the shorthands and a longhand, a
        // colour set before it given up.
        for rule in [
            "border: 1px inset currentcolor",
            "border: 1px inset blue; border-color: currentColor",
            "border: 1px inset blue; border-color: currentcolor blue; \
             border-right-color: currentcolor; border-left-color: currentcolor",
            "border: 1px inset blue; border-top: 1px inset currentcolor; \
             border-right-color: currentcolor; border-bottom-color: currentcolor; \
             border-left-color: currentcolor",
        ] {
            let sheets = [sheet(&format!("p {{ color: red; {rule} }}"))];
            let s = style_of("<p>x</p>", &sheets, "p");
            assert_eq!(s.border_color, [red; 4], "{rule}");
            assert_eq!(current(&s), [true; 4], "{rule}");
            assert_eq!(s.border_style[TOP], BorderStyle::Inset, "{rule}");
        }
    }

    #[test]
    fn borders_are_per_side_with_every_style_and_the_width_keywords() {
        let sheets = [sheet(
            "p { border-style: solid dashed double; border-color: red blue;
                 border-width: thin medium thick 7px; border-left: dotted green; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        use BorderStyle::{Dashed, Dotted, Double, Solid};
        assert_eq!(s.border_style, [Solid, Dashed, Double, Dotted]);
        assert_eq!(
            s.border_color,
            [
                Color::rgb8(255, 0, 0),
                Color::rgb8(0, 0, 255),
                Color::rgb8(255, 0, 0),
                Color::rgb8(0, 128, 0),
            ]
        );
        assert_eq!(
            s.border_width,
            [1.0, 3.0, 5.0, 3.0],
            "border-left reset its width to medium"
        );
        // The ten styles parse; hidden paints nothing; a side shorthand
        // resets the parts it omits, and the longhands set one side.
        for (keyword, style) in [
            ("none", BorderStyle::None),
            ("hidden", BorderStyle::Hidden),
            ("groove", BorderStyle::Groove),
            ("ridge", BorderStyle::Ridge),
            ("inset", BorderStyle::Inset),
            ("outset", BorderStyle::Outset),
        ] {
            let sheets = [sheet(&format!("p {{ border-top-style: {keyword}; }}"))];
            assert_eq!(
                style_of("<p>x</p>", &sheets, "p").border_style[TOP],
                style,
                "{keyword}"
            );
        }
        let sheets = [sheet(
            "p { border: 4px solid red; border-top: dashed; border-right-width: 9px;
                 border-bottom-color: blue; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [3.0, 9.0, 4.0, 4.0]);
        assert_eq!(s.border_style[TOP], Dashed);
        assert_eq!(
            s.border_color[TOP],
            Color::CURRENT,
            "border-top without a color: currentcolor"
        );
        assert_eq!(s.border_color[BOTTOM], Color::rgb8(0, 0, 255));
        assert_eq!(s.used_border_width(), [3.0, 9.0, 4.0, 4.0]);
        let sheets = [sheet("p { border: 4px solid; border-top-style: hidden; }")];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "p").used_border_width(),
            [0.0, 4.0, 4.0, 4.0]
        );
        // The css-wide keywords reach the per-side longhands.
        let sheets = [sheet(
            "div { border-left: 2px dotted red; } p { border-left: inherit; }",
        )];
        let s = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!((s.border_width[LEFT], s.border_style[LEFT]), (2.0, Dotted));
        assert_eq!(s.border_color[LEFT], Color::rgb8(255, 0, 0));
    }

    #[test]
    fn border_none_computes_to_no_visible_border_regardless_of_width() {
        // CSS2 §8.5.3: `border-style: none` means the border doesn't
        // paint even if a width was given — this engine still records
        // `border_width` faithfully; `border_style` is what callers must
        // check before treating that width as visible.
        let sheets = [sheet("p { border-width: 5px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [5.0; 4]);
        assert_eq!(s.border_style, [BorderStyle::None; 4]);
        assert_eq!(s.used_border_width(), [0.0; 4]);
    }

    #[test]
    fn unrecognized_properties_and_values_are_ignored_not_errors() {
        let sheets = [sheet(
            "p { filter: blur(5px); color: red; margin: banana; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0)); // the recognized one still applied
        // The bad value didn't override — `<p>` keeps ua.css's margin,
        // not [0;4] (that would actually be a bug: a bad author value
        // clobbering a previously-cascaded one instead of just not
        // applying).
        assert_eq!(s.margin, px4([16.0, 0.0, 16.0, 0.0]));
    }

    #[test]
    fn unparseable_selector_drops_its_rule_not_the_stylesheet() {
        // M4.10, exercised end to end: a bad selector's rule vanishes;
        // sibling rules in the same sheet still apply.
        let sheets = [sheet(":not() { color: red; } p { color: blue; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn the_ua_sheet_colours_links_and_only_links() {
        // HTML's `:link` colour; an `a` without an `href` is no link.
        let html = "<p><a id=x href=y>link</a><a id=z>anchor</a><area id=w href=v></p>";
        assert_eq!(style_of(html, &[], "#x").color, Color::rgb8(0, 0, 0xEE));
        assert_eq!(
            style_of(html, &[], "#z").color,
            style_of(html, &[], "p").color
        );
        assert_eq!(style_of(html, &[], "#w").color, Color::rgb8(0, 0, 0xEE));
    }

    #[test]
    fn the_ua_sheet_colours_marked_text_by_its_system_colours() {
        let html = "<p style='color: blue'><mark>x</mark></p>";
        let mark = style_of(html, &[], "mark");
        assert_eq!(mark.background_color, Color::rgb8(255, 255, 0));
        assert_eq!(mark.color, Color::rgb8(0, 0, 0));
    }

    // ---- M16: pseudo-element styles, `content` and `quotes` ---------------

    #[test]
    fn pseudo_element_styles_are_kept_beside_the_element() {
        let sheets = [sheet(
            "p { color: #00f } p::before { content: 'x'; color: #f00 }\
             p::first-line { color: #0f0 } p:first-letter { color: #ff0 }\
             p::marker { color: #0ff } p::selection { color: #0ff } p::-webkit-x { color: #0ff }",
        )];
        let html = "<p>text</p><div>no rule</div>";
        let of = |pseudo| pseudo_of(html, &sheets, "p", pseudo);
        assert_eq!(style_of(html, &sheets, "p").color, Color::rgb8(0, 0, 255));
        let before = of(PseudoElement::Before).expect("::before is styled");
        assert_eq!(before.color, Color::rgb8(255, 0, 0));
        assert_eq!(before.content, items(&[text("x")]));
        assert_eq!(of(PseudoElement::After), None, "no rule styles ::after");
        assert_eq!(
            of(PseudoElement::FirstLine).unwrap().color,
            Color::rgb8(0, 255, 0)
        );
        assert_eq!(
            of(PseudoElement::FirstLetter).unwrap().color,
            Color::rgb8(255, 255, 0)
        );
        // Matched and set aside.
        for pseudo in [
            PseudoElement::Marker,
            PseudoElement::Selection,
            PseudoElement::Placeholder,
            PseudoElement::WebkitPrefixed,
        ] {
            assert_eq!(of(pseudo), None);
        }
        let document = Document::from_node(&html::parse_document(html).unwrap());
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        assert!(styles.pseudo_styles(first(&document, "div")).is_none());
        assert!(styles.pseudo_styles(first(&document, "p")).is_some());
    }

    #[test]
    fn a_pseudo_element_inherits_from_its_element_and_cascades_on_its_own() {
        let sheets = [sheet(
            "p { color: #00f; font-size: 20px; padding: 3px }\
             p::after { content: '!'; padding-left: 1em }\
             ::after { margin-left: 7px; padding-left: 2px }\
             .a::after { margin-left: 9px }",
        )];
        let html = "<p class=a style='padding-right: 5px; color: #f00'>x</p>";
        let after = pseudo_of(html, &sheets, "p", PseudoElement::After).unwrap();
        // Inherited from the element, the inline style's colour with it.
        assert_eq!(after.color, Color::rgb8(255, 0, 0));
        assert_eq!(after.font_size_px, 20.0);
        // Its own declarations, `em` against the inherited font size,
        // more specific beating less: (0,0,2) over (0,0,1).
        assert_eq!(after.padding[LEFT], LengthPercentage::Px(20.0));
        assert_eq!(after.margin[LEFT], LengthPercentageAuto::Px(9.0));
        // Not inherited, and the inline style is the element's alone.
        assert_eq!(after.padding[TOP], LengthPercentage::Px(0.0));
        assert_eq!(after.padding[RIGHT], LengthPercentage::Px(0.0));
    }

    #[test]
    fn a_selector_list_styles_a_pseudo_element_with_its_most_specific_branch() {
        // (1,0,1) from `#x::before`, not the first branch's (0,0,2):
        // more specific than `.y::before`'s (0,1,1).
        let sheets = [sheet(
            "p::before, #x::before { content: ''; color: #f00 } .y::before { color: #0f0 }",
        )];
        let before = pseudo_of("<p id=x class=y>x</p>", &sheets, "p", PseudoElement::Before);
        assert_eq!(before.unwrap().color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn a_selector_list_styles_each_target_with_its_own_specificity() {
        // `#x::before` is the more specific in its list for ::before, `p`
        // the one for the element; each competes on its own.
        let sheets = [sheet(
            "p, #x::before { color: #f00 } p#x { color: #00f }\
             p::before { content: ''; color: #0f0 }",
        )];
        let html = "<p id=x>x</p>";
        assert_eq!(style_of(html, &sheets, "p").color, Color::rgb8(0, 0, 255));
        let before = pseudo_of(html, &sheets, "p", PseudoElement::Before).unwrap();
        assert_eq!(before.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn content_applies_to_before_and_after_only() {
        // `normal` computes to `none` on ::before and ::after.
        let sheets = [sheet(
            "p { content: 'element' } p::before { color: #f00 } p::after { content: normal }\
             p::first-line { content: 'x' }",
        )];
        let html = "<p>x</p>";
        let of = |pseudo| pseudo_of(html, &sheets, "p", pseudo).unwrap().content;
        assert_eq!(of(PseudoElement::Before), Content::None);
        assert_eq!(of(PseudoElement::After), Content::None);
        assert_eq!(of(PseudoElement::FirstLine), Content::Normal);
        assert_eq!(style_of(html, &sheets, "p").content, Content::Normal);
        // An image alone is an element's (M23.3).
        let sheets = [sheet(
            "p { content: url(a.png) / 'alt' } q { content: url(a.png) 'x' }",
        )];
        assert!(matches!(
            &style_of("<p>x</p>", &sheets, "p").content,
            Content::Items(items) if matches!(**items, [ContentItem::Image(_)])
        ));
        assert_eq!(style_of("<q>x</q>", &sheets, "q").content, Content::Normal);
        // So `inherit` takes the element's `normal`, and `initial` is
        // `normal`: both `none` here.
        assert_eq!(content("<p>x</p>", "inherit"), Content::None);
        assert_eq!(content("<p>x</p>", "initial"), Content::None);
        assert_eq!(content("<p>x</p>", "unset"), Content::None);
        assert_eq!(content("<p>x</p>", "none"), Content::None);
        assert_eq!(content("<p>x</p>", "NORMAL"), Content::None);
    }

    #[test]
    fn content_takes_strings_attributes_urls_and_quotes() {
        let p = "<p title=T data-x='a b'>x</p>";
        let image = |url: &str| {
            ContentItem::Image(crate::computed::Image::Url {
                url: Arc::from(url),
                resolution: 1.0,
            })
        };
        assert_eq!(
            content(
                p,
                "'a' attr(title) url(x.png) url('y') OPEN-QUOTE close-quote no-open-quote no-close-quote"
            ),
            items(&[
                text("a"),
                text("T"),
                image("x.png"),
                image("y"),
                ContentItem::OpenQuote,
                ContentItem::CloseQuote,
                ContentItem::NoOpenQuote,
                ContentItem::NoCloseQuote,
            ])
        );
        // `attr()`'s name in ASCII lowercase; a missing one is empty.
        assert_eq!(
            content(p, "attr( DATA-X ) attr(nope)"),
            items(&[text("a b"), text("")])
        );
        // `attr()` substitutes (M21.5): a fallback when there is no
        // attribute; a value no `content` takes is invalid at computed-value
        // time, `content` its initial `normal`, which is `none` here.
        assert_eq!(
            content(p, "attr(x, 'fallback') attr(title, 'no')"),
            items(&[text("fallback"), text("T")])
        );
        for invalid in [
            "attr(x string)",
            "attr(title number)",
            "attr(x type(<string>))",
        ] {
            assert_eq!(content(p, invalid), Content::None, "{invalid}");
        }
        // The counter functions (M21.4), kept for the box tree to write:
        // a predefined style's name lowercased, any other kept as written.
        let counter = |name: &str, style: &str| ContentItem::Counter {
            name: Arc::from(name),
            style: Arc::from(style),
        };
        assert_eq!(
            content(
                p,
                "counter(Item) counter(x, UPPER-ROMAN) counter(x, My-Style) counter(x, none)"
            ),
            items(&[
                counter("Item", "decimal"),
                counter("x", "upper-roman"),
                counter("x", "My-Style"),
                counter("x", "none"),
            ])
        );
        assert_eq!(
            content(p, "counters( x , '.' ) counters(x, '-', lower-alpha)"),
            items(&[
                ContentItem::Counters {
                    name: Arc::from("x"),
                    separator: ".".into(),
                    style: Arc::from("decimal"),
                },
                ContentItem::Counters {
                    name: Arc::from("x"),
                    separator: "-".into(),
                    style: Arc::from("lower-alpha"),
                },
            ])
        );
        assert_eq!(content(p, "'a' / counter(x)"), items(&[text("a")]));
        // An image is kept (M23.3); an image-set() with no option this
        // engine shows is left out: a box with nothing in it.
        assert_eq!(content(p, "url(x.png)"), items(&[image("x.png")]));
        assert_eq!(
            content(p, "image-set('x.avif' type('image/avif'))"),
            items(&[])
        );
        assert_eq!(content(p, "url(x.png) / 'alt'"), items(&[image("x.png")]));
        // Alternative text is accepted and set aside.
        assert_eq!(content(p, "'a' / 'alt' attr(title)"), items(&[text("a")]));
        // Anything else drops the declaration: the earlier one stands.
        for bad in [
            "counter()",
            "counter(none)",
            "counter(inherit)",
            "counter('x')",
            "counter(x, 'y')",
            "counter(x,)",
            "counter(x, decimal, decimal)",
            "counter(x, symbols('*'))",
            "counters(x)",
            "counters(x, y)",
            "counters(x, '.', '-')",
            "counters(x, '.', a b)",
            "normal 'a'",
            "none none",
            "attr()",
            "attr('x')",
            "'a' /",
            "/ 'a'",
            "'a' / open-quote",
            "'a' / url(x)",
            "'a' / 'b' / 'c'",
            "'a' b",
            "1",
            "url(x y)",
            "url('x' 'y')",
            // A string a newline ends: a bad string.
            "'bad\n",
        ] {
            assert_eq!(content(p, bad), items(&[text("before")]), "{bad}");
        }
    }

    #[test]
    fn quotes_parse_inherit_and_give_their_marks() {
        let sheets = [sheet(
            "div { quotes: '«' '»' '‹' '›' } .none { quotes: NONE } .auto { quotes: auto }\
             .odd { quotes: 'a' } .ident { quotes: a b } .mixed { quotes: 'a' b }\
             .unset { quotes: unset }",
        )];
        let html = "<div><p>x</p><p class=none>x</p><p class=auto>x</p>\
                    <p class=odd>x</p><p class=ident>x</p><p class=mixed>x</p>\
                    <p class=unset>x</p></div>";
        let pairs = Quotes::Pairs(Arc::from(vec![
            ("«".to_owned(), "»".to_owned()),
            ("‹".to_owned(), "›".to_owned()),
        ]));
        assert_eq!(style_of(html, &sheets, "p").quotes, pairs, "inherited");
        assert_eq!(style_of(html, &sheets, ".none").quotes, Quotes::None);
        assert_eq!(style_of(html, &sheets, ".auto").quotes, Quotes::Auto);
        assert_eq!(style_of(html, &sheets, ".unset").quotes, pairs, "inherited");
        for bad in [".odd", ".ident", ".mixed"] {
            assert_eq!(style_of(html, &sheets, bad).quotes, pairs, "{bad}");
        }
        assert_eq!(pairs.marks(0), Some(("«", "»")));
        assert_eq!(pairs.marks(1), Some(("‹", "›")));
        assert_eq!(
            pairs.marks(5),
            Some(("‹", "›")),
            "past the last pair, the last"
        );
        assert_eq!(Quotes::Auto.marks(0), Some(("\u{201C}", "\u{201D}")));
        assert_eq!(Quotes::Auto.marks(3), Some(("\u{2018}", "\u{2019}")));
        assert_eq!(Quotes::None.marks(0), None);
        assert_eq!(ComputedStyle::default().quotes, Quotes::Auto);
    }

    #[test]
    fn the_ua_sheet_quotes_q() {
        let html = "<p><q>x</q></p>";
        let of = |pseudo| pseudo_of(html, &[], "q", pseudo).unwrap().content;
        assert_eq!(of(PseudoElement::Before), items(&[ContentItem::OpenQuote]));
        assert_eq!(of(PseudoElement::After), items(&[ContentItem::CloseQuote]));
        assert_eq!(pseudo_of(html, &[], "p", PseudoElement::Before), None);
    }

    #[test]
    fn a_rule_for_a_pseudo_element_does_not_style_its_originating_element() {
        // M16: `p::before` represents the pseudo-element, not `p`; and
        // since it parses, the other selectors in its list apply where
        // they used to be dropped with it.
        let sheets = [sheet(
            "p { color: blue; } p::before, p:after { color: red; } \
             .a, p::first-line, b::-webkit-scrollbar { color: green; }",
        )];
        let html = "<p>x</p><b class=a>y</b>";
        assert_eq!(style_of(html, &sheets, "p").color, Color::rgb8(0, 0, 255));
        assert_eq!(style_of(html, &sheets, "b").color, Color::rgb8(0, 128, 0));
    }

    #[test]
    fn display_none_is_recorded_but_children_still_get_styles() {
        let sheets = [sheet("div { display: none; } span { color: red; }")];
        let document =
            Document::from_node(&html::parse_document("<div><span>x</span></div>").unwrap());
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let style = |tag: &str| styles.get(first(&document, tag)).expect("styled");
        assert_eq!(style("div").display, Display::None);
        // Styling doesn't itself decide what's painted — that's paint's
        // job, walking `display` — so a display:none subtree's children
        // still have well-defined styles.
        assert_eq!(style("span").color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn a_first_letter_takes_only_the_properties_that_apply_to_it() {
        // Its fonts, colour, background, spacing, margins, padding,
        // borders and `float` apply; `width`, `display`, `position`,
        // `content`, `white-space` and a table's borders do not, so the
        // inherited ones stay the element's and the others initial.
        let sheets = [sheet(
            "p { white-space: pre-wrap; border-spacing: 1px; }
             p::first-letter { color: #f00; background-color: #0f0; font-size: 30px;
                 font-family: serif; text-transform: uppercase; letter-spacing: 2px;
                 word-spacing: 3px; vertical-align: top; margin: 2px; padding-left: 3px;
                 border: 1px solid #00f; float: left; width: 100px; display: inline-block;
                 position: absolute; content: 'x'; white-space: normal; overflow: hidden;
                 border-collapse: collapse; border-spacing: 9px; }",
        )];
        let document = Document::from_node(&html::parse_document("<p>x</p>").unwrap());
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let s = styles
            .pseudo(first(&document, "p"), PseudoElement::FirstLetter)
            .expect("styled");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
        assert_eq!(s.background_color, Color::rgb8(0, 255, 0));
        assert_eq!((s.font_size_px, &*s.font().family), (30.0, "serif"));
        assert_eq!(s.text_transform, TextTransform::Uppercase);
        assert_eq!((s.letter_spacing, s.word_spacing), (2.0, 3.0));
        assert_eq!(s.vertical_align, VerticalAlign::Top);
        assert_eq!(s.margin[TOP], LengthPercentageAuto::Px(2.0));
        assert_eq!(s.padding[LEFT], LengthPercentage::Px(3.0));
        assert_eq!(s.border_style, [BorderStyle::Solid; 4]);
        assert_eq!(s.float, Float::Left);
        // A float's `display` is blockified from the initial `inline`.
        assert_eq!(s.display, Display::Block);
        assert_eq!(s.width, Size::Auto);
        assert_eq!(s.position, Position::Static);
        assert_eq!(s.content, Content::Normal);
        assert_eq!(s.white_space, WhiteSpace::PRE_WRAP);
        assert_eq!(s.overflow(), Overflow::Visible);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
        assert_eq!(
            (s.border_spacing.horizontal, s.border_spacing.vertical),
            (1.0, 1.0)
        );
    }

    #[test]
    fn the_rules_that_matched_are_given_in_cascade_order_with_what_they_lost() {
        let sheets = Stylesheets::new(vec![
            sheet(
                "p { color: red; margin: 1px }                  @media screen { @layer base { p.x { color: green } } }                  #i, .never { color: blue !important; padding-top: 2px }                  div { color: gray } p::first-line { color: pink }",
            ),
            sheet("p { margin-top: 3px }"),
        ]);
        let document = Document::from_node(
            &html::parse_document("<p id=i class=x style='padding-top: 9px; color: black'>x</p>")
                .unwrap(),
        );
        let context = CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let p = first(&document, "p");
        let rules = sheets
            .recascader(&document, &styles, &context)
            .matched_rules(p);
        let names: Vec<(RuleOrigin, &str)> = rules
            .iter()
            .map(|rule| (rule.origin, rule.selector.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                (RuleOrigin::Inline, ""),
                (RuleOrigin::Author(0), "#i, .never"),
                (RuleOrigin::Author(1), "p"),
                (RuleOrigin::Author(0), "p"),
                (RuleOrigin::Author(0), "p.x"),
                // The UA sheet's isolates (HTML section 15.3.5).
                (
                    RuleOrigin::UserAgent,
                    "address, blockquote, center, div, figure, figcaption, footer, form, \
                     header, hr, legend, listing, main, p, plaintext, pre, summary, xmp, \
                     article, aside, h1, h2, h3, h4, h5, h6, hgroup, nav, section, search, \
                     table, caption, colgroup, col, thead, tbody, tfoot, tr, td, th, dir, dd, \
                     dl, dt, menu, ol, ul, li, bdi, output, [dir=ltr i], [dir=rtl i], \
                     [dir=auto i]"
                ),
                (RuleOrigin::UserAgent, "p"),
            ],
            "{rules:#?}"
        );
        let decl = |rule: usize, name: &str| {
            rules[rule]
                .declarations
                .iter()
                .find(|d| d.name == name)
                .unwrap_or_else(|| panic!("{name} in {:?}", rules[rule]))
                .clone()
        };
        // The important color wins over the inline one; the inline
        // padding-top over the rule's, which is not important.
        assert!(decl(0, "color").overridden);
        assert!(!decl(1, "color").overridden && decl(1, "color").important);
        assert!(!decl(0, "padding-top").overridden);
        assert!(decl(1, "padding-top").overridden);
        assert_eq!(rules[1].specificity, (1, 0, 0));
        // The later sheet's margin-top leaves the shorthand the rest.
        assert!(!decl(2, "margin-top").overridden);
        assert!(!decl(3, "margin").overridden);
        assert_eq!(decl(3, "margin").value, "1px");
        // A layered rule loses to the unlayered ones, and says where it is.
        assert!(decl(4, "color").overridden);
        assert_eq!(rules[4].conditions, ["@media screen", "@layer base"]);
        // An image's width attribute is a hint.
        let document = Document::from_node(&html::parse_document("<img width=7>").unwrap());
        let styles = sheets.cascade(&document, &context);
        let img = first(&document, "img");
        let rules = sheets
            .recascader(&document, &styles, &context)
            .matched_rules(img);
        let hints = rules
            .iter()
            .find(|rule| rule.origin == RuleOrigin::Hints)
            .expect("hints");
        assert_eq!(hints.declarations[0].name, "width");
        assert_eq!(hints.declarations[0].value, "7px");
    }

    #[test]
    fn a_recascade_gives_a_pseudo_elements_style_from_another_parent() {
        let sheets = Stylesheets::new(vec![sheet(
            "p { font-size: 16px; color: #00f; } p::first-letter { font-size: 2em; }",
        )]);
        let document = Document::from_node(&html::parse_document("<p>x</p><div>y</div>").unwrap());
        let context = CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let p = first(&document, "p");
        let cascaded = styles.pseudo(p, PseudoElement::FirstLetter).unwrap();
        assert_eq!(
            (cascaded.font_size_px, cascaded.color),
            (32.0, Color::rgb8(0, 0, 255))
        );
        let recascade = sheets.recascader(&document, &styles, &context);
        let parent = ComputedStyle {
            font_size_px: 10.0,
            color: Color::rgb8(1, 2, 3),
            ..ComputedStyle::default()
        };
        // `em` and the inherited colour are the given parent's.
        let s = recascade
            .pseudo_element(p, PseudoElement::FirstLetter, &parent)
            .unwrap();
        assert_eq!((s.font_size_px, s.color), (20.0, Color::rgb8(1, 2, 3)));
        // Nothing styles the div's, nor the p's `::first-line`, and the
        // p is no list item to have a marker.
        let div = first(&document, "div");
        assert!(
            recascade
                .pseudo_element(div, PseudoElement::FirstLetter, &parent)
                .is_none()
        );
        assert!(
            recascade
                .pseudo_element(p, PseudoElement::FirstLine, &parent)
                .is_none()
        );
        assert!(
            recascade
                .pseudo_element(p, PseudoElement::Marker, &parent)
                .is_none()
        );
        // With the element's own style as parent, it is the one the
        // cascade kept, or none.
        assert_eq!(
            recascade.pseudo_element(p, PseudoElement::FirstLetter, styles.get(p).unwrap()),
            Some(cascaded.clone())
        );
        assert!(
            recascade
                .pseudo_element(div, PseudoElement::FirstLetter, styles.get(div).unwrap())
                .is_none()
        );
    }

    #[test]
    fn a_recascade_resolves_rem_as_the_cascade_does() {
        // The root's `rem` is the initial font size, every other
        // element's the root's.
        let sheets = Stylesheets::new(vec![sheet(
            "html { font-size: 20px; } ::first-letter { font-size: 2rem; }",
        )]);
        let document = Document::from_node(&html::parse_document("<p>x</p>").unwrap());
        let context = CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let size = |tag: &str| {
            let element = first(&document, tag);
            // The element's style but for a colour, so that the rules are
            // matched again rather than the cascade's style taken.
            let parent = ComputedStyle {
                color: Color::rgb8(1, 2, 3),
                ..styles.get(element).expect("styled").clone()
            };
            let cascaded = styles.pseudo(element, PseudoElement::FirstLetter).unwrap();
            let recascaded = recascade
                .pseudo_element(element, PseudoElement::FirstLetter, &parent)
                .unwrap();
            (cascaded.font_size_px, recascaded.font_size_px)
        };
        assert_eq!(size("html"), (32.0, 32.0));
        assert_eq!(size("p"), (40.0, 40.0));
    }

    #[test]
    fn a_first_line_takes_only_the_properties_that_apply_to_it() {
        // Its fonts, colour, background, spacing and `vertical-align`
        // apply; margins, padding, borders, `float`, `display` and
        // `white-space` do not.
        let sheets = [sheet(
            "p::first-line { color: #f00; background-color: #0f0; font-size: 30px;
                 font-family: serif; text-transform: uppercase; letter-spacing: 2px;
                 word-spacing: 3px; vertical-align: top; margin: 2px; padding-left: 3px;
                 border: 1px solid #00f; float: left; display: block; white-space: pre; }",
        )];
        let document = Document::from_node(&html::parse_document("<p>x</p>").unwrap());
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let s = styles
            .pseudo(first(&document, "p"), PseudoElement::FirstLine)
            .expect("styled");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
        assert_eq!(s.background_color, Color::rgb8(0, 255, 0));
        assert_eq!((s.font_size_px, &*s.font().family), (30.0, "serif"));
        assert_eq!(s.text_transform, TextTransform::Uppercase);
        assert_eq!((s.letter_spacing, s.word_spacing), (2.0, 3.0));
        assert_eq!(s.vertical_align, VerticalAlign::Top);
        let initial = ComputedStyle::default();
        assert_eq!(
            (&s.margin, &s.padding, &s.border_style),
            (&initial.margin, &initial.padding, &initial.border_style)
        );
        assert_eq!(
            (s.float, s.display, s.white_space),
            (Float::None, Display::Inline, WhiteSpace::NORMAL)
        );
    }

    #[test]
    fn a_recascade_gives_an_elements_style_from_another_parent() {
        // The span's own rules, over the given parent: its `em` and what
        // it inherits are the parent's, what it sets its own.
        let sheets = Stylesheets::new(vec![sheet(
            "p { color: #00f } span { font-size: 2em; } .own { color: #0f0 }",
        )]);
        let document = Document::from_node(
            &html::parse_document(
                "<p><span>a</span><span class=own style='padding-left: 3px'>b</span></p>",
            )
            .unwrap(),
        );
        let context = CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let parent = ComputedStyle {
            font_size_px: 10.0,
            color: Color::rgb8(1, 2, 3),
            ..ComputedStyle::default()
        };
        let spans: Vec<NodeId> = document
            .subtree(document.root())
            .filter(|&id| document.tag(id) == Some("span"))
            .collect();
        let s = recascade.element(spans[0], &parent);
        assert_eq!((s.font_size_px, s.color), (20.0, Color::rgb8(1, 2, 3)));
        // A class rule and the `style` attribute are its rules too.
        let s = recascade.element(spans[1], &parent);
        assert_eq!(s.color, Color::rgb8(0, 255, 0));
        assert_eq!(s.padding[LEFT], LengthPercentage::Px(3.0));
        // The cascade's own, from the p's style, is the same as asked
        // for with that parent.
        let p = first(&document, "p");
        assert_eq!(
            recascade.element(spans[0], styles.get(p).unwrap()),
            *styles.get(spans[0]).unwrap()
        );
    }

    #[test]
    fn the_root_elements_display_is_blockified() {
        let document =
            Document::from_node(&html::parse_document("<div><span>x</span></div>").unwrap());
        // The root's, and another element's, which is what it says.
        for (value, root, other) in [
            ("inline", Display::Block, Display::Inline),
            ("inline-block", Display::Block, Display::InlineBlock),
            ("table-cell", Display::Block, Display::TableCell),
            ("inline-table", Display::Table, Display::InlineTable),
            ("none", Display::None, Display::None),
        ] {
            let sheets = [sheet(&format!("html, span {{ display: {value}; }}"))];
            let styles = cascade_document(&document, &sheets, &CascadeContext::default());
            let style = |tag: &str| styles.get(first(&document, tag)).expect("styled");
            assert_eq!(
                (style("html").display, style("span").display),
                (root, other),
                "{value}"
            );
        }
    }

    #[test]
    fn a_legend_is_blockified_and_a_fieldset_takes_the_ua_sheets_box() {
        let document = Document::from_node(
            &html::parse_document(
                "<fieldset><legend>x</legend></fieldset><div><legend>y</legend></div>",
            )
            .unwrap(),
        );
        let sheets = [sheet(
            "legend { display: inline-flex } div legend { display: inline }",
        )];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let legends: Vec<Display> = document
            .subtree(document.root())
            .filter(|&n| document.tag(n) == Some("legend"))
            .map(|n| styles.get(n).expect("styled").display)
            .collect();
        assert_eq!(legends, [Display::Flex, Display::Block]);
        let fieldset = styles.get(first(&document, "fieldset")).expect("styled");
        assert!(fieldset.fieldset && !styles.get(first(&document, "div")).unwrap().fieldset);
        // HTML section 15.3.13's box, at 16px.
        assert_eq!(fieldset.margin[LEFT], LengthPercentageAuto::Px(2.0));
        assert_eq!(fieldset.margin[TOP], LengthPercentageAuto::Px(0.0));
        assert_eq!(fieldset.padding[TOP], LengthPercentage::Px(0.35 * 16.0));
        assert_eq!(fieldset.padding[BOTTOM], LengthPercentage::Px(10.0));
        assert_eq!(fieldset.padding[RIGHT], LengthPercentage::Px(12.0));
        assert_eq!(fieldset.border_width, [2.0; 4]);
        assert_eq!(fieldset.border_style, [BorderStyle::Groove; 4]);
        assert_eq!(fieldset.min_width, Size::MinContent);
        let legend = styles.get(first(&document, "legend")).expect("styled");
        assert_eq!(legend.padding[LEFT], LengthPercentage::Px(2.0));
    }

    #[test]
    fn the_flow_relative_box_properties_set_their_physical_sides() {
        let document = Document::from_node(
            &html::parse_document("<p>x</p><p dir=rtl>y</p><div><span>z</span></div>").unwrap(),
        );
        let sheets = [sheet(
            "p { margin-inline: 1px 2px; margin-block-end: 3px; padding-block: 4px;
                 padding-inline-start: 5px; min-inline-size: 6px; max-block-size: 7px;
                 inline-size: 8px; block-size: 9px }
             div { margin-inline: 10px; padding-inline: 1px 2px 3px }
             span { display: block; margin-inline-start: inherit; min-block-size: min-content }",
        )];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let px = LengthPercentageAuto::Px;
        let ps: Vec<NodeId> = document
            .subtree(document.root())
            .filter(|&n| document.tag(n) == Some("p"))
            .collect();
        for (p, [start, end]) in ps.into_iter().zip([[LEFT, RIGHT], [RIGHT, LEFT]]) {
            let s = styles.get(p).expect("styled");
            assert_eq!((&s.margin[start], &s.margin[end]), (&px(1.0), &px(2.0)));
            assert_eq!(s.margin[BOTTOM], px(3.0));
            assert_eq!(s.padding[TOP], LengthPercentage::Px(4.0));
            assert_eq!(s.padding[BOTTOM], LengthPercentage::Px(4.0));
            assert_eq!(s.padding[start], LengthPercentage::Px(5.0));
            assert_eq!(s.min_width, Size::Px(6.0));
            assert_eq!(s.width, Size::Px(8.0));
            assert_eq!(s.height, Size::Px(9.0));
            assert_eq!(
                crate::serialize::computed_value(s, "max-block-size").as_deref(),
                Some("7px")
            );
            assert_eq!(
                crate::serialize::computed_value(s, "margin-inline-end").as_deref(),
                Some("2px")
            );
        }
        let div = styles.get(first(&document, "div")).expect("styled");
        assert_eq!(
            (&div.margin[LEFT], &div.margin[RIGHT]),
            (&px(10.0), &px(10.0))
        );
        // Three values are not a two-sided shorthand's.
        assert_eq!(div.padding[LEFT], LengthPercentage::Px(0.0));
        let span = styles.get(first(&document, "span")).expect("styled");
        assert_eq!(span.margin[LEFT], px(10.0));
        assert_eq!(span.min_height, Size::MinContent);
        assert!(supports("padding-block", &css::tokenize("1px 2px")));
        assert!(!supports("margin-inline-start", &css::tokenize("1px 2px")));
    }

    #[test]
    fn direction_is_a_property_the_flow_relative_ones_follow() {
        // D74: `direction` set by a rule, whatever its order with the
        // flow-relative declarations, maps them; a later physical
        // declaration wins over an earlier flow-relative one and the
        // other way round; `dir` sets it through the user agent's sheet,
        // which a rule overrides.
        let document = Document::from_node(
            &html::parse_document(
                "<p id=a>x</p><p id=b dir=rtl>y</p><div id=c dir=rtl><p id=d>z</p></div>",
            )
            .unwrap(),
        );
        let sheets = [sheet(
            "#a { inset-inline-start: 1px; border-inline-end: 2px solid; direction: rtl;
                  border-start-end-radius: 3px; margin-left: 4px; margin-inline-start: 5px }
             #b { direction: ltr; padding-inline: 1px 2px; padding-left: 3px }
             #d { float: inline-start; clear: inline-end; text-align: end }",
        )];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let style = |id: &str| styles.get(by_id(&document, id)).expect("styled");
        let a = style("a");
        assert_eq!(a.direction, Direction::Rtl);
        assert_eq!(a.inset[RIGHT], LengthPercentageAuto::Px(1.0));
        assert_eq!(a.border_width[LEFT], 2.0);
        assert_eq!(a.border_style[LEFT], BorderStyle::Solid);
        assert_eq!(a.margin[RIGHT], LengthPercentageAuto::Px(5.0));
        assert_eq!(a.margin[LEFT], LengthPercentageAuto::Px(4.0));
        let value = |s: &ComputedStyle, name: &str| crate::serialize::computed_value(s, name);
        assert_eq!(value(a, "border-top-left-radius").as_deref(), Some("3px"));
        assert_eq!(
            value(a, "border-inline-end").as_deref(),
            Some("2px solid rgb(0, 0, 0)")
        );
        let b = style("b");
        assert_eq!(b.direction, Direction::Ltr, "a rule over `dir`");
        assert_eq!(b.padding[LEFT], LengthPercentage::Px(3.0));
        assert_eq!(b.padding[RIGHT], LengthPercentage::Px(2.0));
        assert_eq!(value(b, "padding-inline").as_deref(), Some("3px 2px"));
        // The flow-relative values by the parent's direction, kept as
        // written.
        let d = style("d");
        assert_eq!(d.direction, Direction::Rtl);
        assert_eq!((d.float, d.clear), (Float::Right, Clear::Left));
        assert_eq!(value(d, "float").as_deref(), Some("inline-start"));
        assert_eq!(value(d, "clear").as_deref(), Some("inline-end"));
        assert_eq!(value(d, "text-align").as_deref(), Some("end"));
        assert!(supports("resize", &css::tokenize("block")));
        assert!(supports("direction", &css::tokenize("rtl")));
        assert!(!supports("direction", &css::tokenize("auto")));
    }

    /// The first element whose `id` is `id`.
    fn by_id(document: &Document, id: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("#{id}"))
    }

    #[test]
    fn container_queries_read_the_sizes_laid_out_and_the_styles() {
        // D72: a size query of a container not laid out is unknown; with
        // its size, it holds or not; a restyle with new sizes changes the
        // subtree; style queries read the container's custom properties;
        // the container units its content box, the viewport's without one.
        let mut document = Document::from_node(
            &html::parse_document(
                "<div id=c style='--theme: dark'><div><p id=p>x</p><span id=s>y</span></div></div>\
                 <i id=i>z</i>",
            )
            .unwrap(),
        );
        let sheets = Stylesheets::new(vec![sheet(
            "#c { container: card / inline-size }
             @container card (width > 100px) { #p { color: #f00 } }
             @container (height > 1px) { #p { color: #0f0 } }
             @container style(--theme: dark) { #s { color: #00f } }
             #s { width: 50cqw } #i { width: 10cqi }",
        )]);
        let context = CascadeContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            ..CascadeContext::default()
        };
        let mut styles = sheets.cascade(&document, &context);
        let (c, p, s, i) = (
            by_id(&document, "c"),
            by_id(&document, "p"),
            by_id(&document, "s"),
            by_id(&document, "i"),
        );
        let black = Color::BLACK;
        assert_eq!(styles.get(p).unwrap().color, black, "unknown, not laid out");
        assert_eq!(styles.get(s).unwrap().color, Color::rgb8(0, 0, 255));
        assert_eq!(styles.get(s).unwrap().width, Size::Px(400.0));
        assert_eq!(styles.get(i).unwrap().width, Size::Px(80.0));
        assert_eq!(styles.queried_containers().collect::<Vec<_>>(), [c]);
        let mut sizes = crate::container::ContainerSizes::new();
        sizes.insert(c, 200.0, 50.0);
        let restyle = sheets
            .restyle_containers(&document, &context, &mut styles, &sizes)
            .expect("its size changed");
        assert!(restyle.changed.contains(&p));
        assert_eq!(styles.get(p).unwrap().color, Color::rgb8(255, 0, 0));
        assert_eq!(styles.get(s).unwrap().width, Size::Px(100.0));
        // Asked again with the same sizes: nothing changed.
        assert!(
            sheets
                .restyle_containers(&document, &context, &mut styles, &sizes)
                .is_none()
        );
        // A restyle keeps the sizes it was given.
        document.set_attribute(s, "class", "x").unwrap();
        let changes = document.take_changes();
        sheets.restyle(&document, &context, &mut styles, &changes);
        assert_eq!(styles.get(p).unwrap().color, Color::rgb8(255, 0, 0));
        // A container's type changed, nothing it gives its children by
        // inheritance: its whole subtree's queries again, the paragraph's
        // under the wrapper among them.
        document
            .set_attribute(c, "style", "--theme: dark; container-type: normal")
            .unwrap();
        let changes = document.take_changes();
        sheets.restyle(&document, &context, &mut styles, &changes);
        assert_eq!(styles.get(p).unwrap().color, black);
        assert_eq!(styles.get(s).unwrap().width, Size::Px(400.0));
        // Its custom property changed: the style queries again.
        document
            .set_attribute(c, "style", "--theme: light")
            .unwrap();
        let changes = document.take_changes();
        sheets.restyle(&document, &context, &mut styles, &changes);
        assert_eq!(styles.get(s).unwrap().color, black);
    }

    #[test]
    fn the_container_properties_parse_and_serialize() {
        let value = |css: &str, name: &str| {
            let document = Document::from_node(&html::parse_document("<p>x</p>").unwrap());
            let styles = cascade_document(&document, &[sheet(css)], &CascadeContext::default());
            crate::serialize::computed_value(styles.get(first(&document, "p")).unwrap(), name)
                .unwrap()
        };
        assert_eq!(
            value("p { container: a b / size }", "container"),
            "a b / size"
        );
        assert_eq!(value("p { container: a / normal }", "container"), "a");
        assert_eq!(
            value("p { container-type: scroll-state size }", "container-type"),
            "size scroll-state"
        );
        assert_eq!(
            value(
                "p { container-type: Anchored scroll-state size }",
                "container-type"
            ),
            "size scroll-state anchored"
        );
        assert_eq!(
            value("p { container-name: none }", "container-name"),
            "none"
        );
        assert_eq!(
            value("p { container-name: FoO foo }", "container-name"),
            "FoO foo"
        );
        for invalid in ["none none", "and", "\"foo\"", "default"] {
            assert!(
                !supports("container-name", &css::tokenize(invalid)),
                "{invalid}"
            );
        }
        for invalid in [
            "size inline-size",
            "normal size",
            "style",
            "anchored anchored",
            "normal anchored",
        ] {
            assert!(
                !supports("container-type", &css::tokenize(invalid)),
                "{invalid}"
            );
        }
    }

    #[test]
    fn display_parses_its_keywords_and_is_not_inherited() {
        let sheets = [sheet("div { display: inline-block; }")];
        let document =
            Document::from_node(&html::parse_document("<div><span>x</span></div>").unwrap());
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let style = |tag: &str| styles.get(first(&document, tag)).expect("styled");
        assert_eq!(style("div").display, Display::InlineBlock);
        assert_eq!(style("span").display, Display::Inline);
        let html = "<div><span>x</span></div>";
        for (value, expected) in [
            ("block", Display::Block),
            ("inline", Display::Inline),
            ("INLINE-BLOCK", Display::InlineBlock),
            ("none", Display::None),
            ("table", Display::Table),
            ("inline-table", Display::InlineTable),
            ("table-row-group", Display::TableRowGroup),
            ("table-header-group", Display::TableHeaderGroup),
            ("table-footer-group", Display::TableFooterGroup),
            ("table-row", Display::TableRow),
            ("table-column-group", Display::TableColumnGroup),
            ("table-column", Display::TableColumn),
            ("Table-Cell", Display::TableCell),
            ("table-caption", Display::TableCaption),
            ("flex", Display::Flex),
            ("inline-flex", Display::InlineFlex),
            // The Compatibility Standard's keywords (§3.4.5).
            ("-webkit-flex", Display::Flex),
            ("-WEBKIT-INLINE-FLEX", Display::InlineFlex),
            ("grid", Display::Grid),
            ("inline-grid", Display::InlineGrid),
        ] {
            let sheets = [sheet(&format!("span {{ display: {value}; }}"))];
            assert_eq!(style_of(html, &sheets, "span").display, expected, "{value}");
        }
        // An unknown keyword leaves the UA value in place, and so does
        // `-webkit-box`, whose mapping the Standard calls not web
        // compatible.
        for value in ["grid-lanes", "-webkit-box", "-webkit-inline-box"] {
            let sheets = [sheet(&format!("div {{ display: {value}; }}"))];
            assert_eq!(
                style_of(html, &sheets, "div").display,
                Display::Block,
                "{value}"
            );
        }
    }

    #[test]
    fn the_children_of_a_flex_container_are_blockified() {
        // css-display-3 §2.7: each child's display as a flex item; a
        // grandchild keeps its own; `none` stays.
        let html = "<div><span>a<b>b</b></span><i>i</i><em>e</em><u>u</u><s>s</s>\
                    <q>q</q><small>n</small><code>c</code></div>";
        let sheets = [sheet(
            "div { display: flex; } i { display: inline-block; } em { display: inline-table; }
             u { display: inline-flex; } s { display: table-cell; } q { display: none; }
             small { float: left; } code { position: absolute; }",
        )];
        for (selector, expected) in [
            ("span", Display::Block),
            ("b", Display::Inline),
            ("i", Display::Block),
            ("em", Display::Table),
            ("u", Display::Flex),
            ("s", Display::Block),
            ("q", Display::None),
            ("small", Display::Block),
            ("code", Display::Block),
        ] {
            assert_eq!(
                style_of(html, &sheets, selector).display,
                expected,
                "{selector}"
            );
        }
        // A floated item keeps its `float`: the box tree ignores it.
        assert_eq!(style_of(html, &sheets, "small").float, Float::Left);
        // An inline flex container's children are items too.
        let sheets = [sheet("div { display: inline-flex; }")];
        assert_eq!(style_of(html, &sheets, "span").display, Display::Block);
        // The container's `::before` and `::after` are items; its
        // `::first-letter` is not one, and does not apply anyway.
        let sheets = [sheet(
            "div { display: flex; } div::before, div::after { content: 'x'; }
             div::first-letter { color: red; }",
        )];
        for pseudo in [PseudoElement::Before, PseudoElement::After] {
            assert_eq!(
                pseudo_of("<div>x</div>", &sheets, "div", pseudo).map(|s| s.display),
                Some(Display::Block),
                "{pseudo:?}"
            );
        }
        assert_eq!(
            pseudo_of("<div>x</div>", &sheets, "div", PseudoElement::FirstLetter)
                .map(|s| s.display),
            Some(Display::Inline)
        );
    }

    #[test]
    fn the_flex_longhands_parse_and_are_not_inherited() {
        let html = "<div><p>x</p></div>";
        let p = |css: &str| style_of(html, &[sheet(css)], "p");
        for (value, expected) in [
            ("row", FlexDirection::Row),
            ("row-reverse", FlexDirection::RowReverse),
            ("Column", FlexDirection::Column),
            ("column-reverse", FlexDirection::ColumnReverse),
        ] {
            let s = p(&format!("p {{ flex-direction: {value} }}"));
            assert_eq!(s.flex_direction, expected, "{value}");
        }
        for (value, expected) in [
            ("nowrap", FlexWrap::NoWrap),
            ("wrap", FlexWrap::Wrap),
            ("wrap-reverse", FlexWrap::WrapReverse),
        ] {
            assert_eq!(
                p(&format!("p {{ flex-wrap: {value} }}")).flex_wrap,
                expected
            );
        }
        for (value, expected) in [
            ("column", (FlexDirection::Column, FlexWrap::NoWrap)),
            ("wrap", (FlexDirection::Row, FlexWrap::Wrap)),
            (
                "wrap-reverse row-reverse",
                (FlexDirection::RowReverse, FlexWrap::WrapReverse),
            ),
            ("column wrap", (FlexDirection::Column, FlexWrap::Wrap)),
        ] {
            let s = p(&format!(
                "p {{ flex-direction: column-reverse; flex-wrap: wrap; flex-flow: {value} }}"
            ));
            assert_eq!((s.flex_direction, s.flex_wrap), expected, "{value}");
        }
        // Invalid: a part twice, an unknown part, three parts.
        for value in ["row column", "wrap nowrap", "row sideways", "row wrap row"] {
            let s = p(&format!("p {{ flex-flow: {value} }}"));
            assert_eq!(
                (s.flex_direction, s.flex_wrap),
                (FlexDirection::Row, FlexWrap::NoWrap),
                "{value}"
            );
        }
        // An empty value is no value: what was set stays.
        let s = p("p { flex-flow: column wrap; flex-flow: ; }");
        assert_eq!(
            (s.flex_direction, s.flex_wrap),
            (FlexDirection::Column, FlexWrap::Wrap)
        );
        assert_eq!(p("p { order: -3 }").order, -3);
        assert_eq!(p("p { order: 2.5 }").order, 0, "an integer only");
        assert_eq!(p("p { order: 1e10 }").order, 0, "a number, not an integer");
        assert_eq!(p("p { order: -17014118346 }").order, i32::MIN, "clamped");
        assert_eq!(p("p { order: 17014118346 }").order, i32::MAX, "clamped");
        let s = p("p { flex-grow: 2.5; flex-shrink: 0 }");
        assert_eq!((s.flex_grow, s.flex_shrink), (2.5, 0.0));
        let s = p("p { flex-grow: -1; flex-shrink: 1px }");
        assert_eq!((s.flex_grow, s.flex_shrink), (0.0, 1.0), "invalid ones");
        assert_eq!(p("p { flex-grow: 1e300 }").flex_grow, f32::MAX);
        for (value, expected) in [
            ("content", FlexBasis::Content),
            ("auto", FlexBasis::Size(Size::Auto)),
            ("10px", FlexBasis::Size(Size::Px(10.0))),
            ("50%", FlexBasis::Size(Size::Percent(50.0))),
            ("0", FlexBasis::Size(Size::Px(0.0))),
            ("max-content", FlexBasis::Size(Size::MaxContent)),
            ("-1px", FlexBasis::Size(Size::Auto)),
            ("none", FlexBasis::Size(Size::Auto)),
        ] {
            let s = p(&format!("p {{ flex-basis: {value} }}"));
            assert_eq!(s.flex_basis, expected, "{value}");
        }
        // None of them inherits from a parent that sets them all.
        let s = style_of(
            html,
            &[sheet(
                "div { flex-flow: column wrap; order: 2; flex: 2 3 4px; }",
            )],
            "p",
        );
        let initial = ComputedStyle::default();
        assert_eq!(
            (s.flex_direction, s.flex_wrap, s.order),
            (initial.flex_direction, initial.flex_wrap, initial.order)
        );
        assert_eq!(
            (s.flex_grow, s.flex_shrink, s.flex_basis),
            (initial.flex_grow, initial.flex_shrink, initial.flex_basis)
        );
    }

    #[test]
    fn the_motion_properties_cascade_with_their_shorthands_and_aliases() {
        let motion = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p").motion().clone();
        let name = |n: &str| Some(Arc::from(n));
        // The shorthand sets every longhand, those it leaves out to their
        // initial values; a longhand after it wins.
        let m = motion(
            "p { animation-delay: 9s; animation-play-state: paused; \
             animation: spin 2s linear infinite; animation-direction: reverse }",
        );
        assert_eq!(m.animation_name, vec![name("spin")]);
        assert_eq!(m.animation_duration, vec![Some(2.0)]);
        assert_eq!(m.animation_delay, vec![0.0]);
        assert_eq!(m.animation_play_state, vec![AnimationPlayState::Running]);
        assert_eq!(m.animation_iteration_count, vec![f64::INFINITY]);
        assert_eq!(m.animation_direction, vec![AnimationDirection::Reverse]);
        // The Compatibility Standard's names are the properties.
        let m = motion(
            "p { -webkit-transition: opacity 1s; -webkit-transition-delay: 2s; \
             -webkit-animation: a 3s; -webkit-animation-fill-mode: both }",
        );
        assert_eq!(
            m.transition_property,
            vec![TransitionProperty::Name(Arc::from("opacity"))]
        );
        assert_eq!(
            (m.transition_duration[0], m.transition_delay[0]),
            (1.0, 2.0)
        );
        assert_eq!(m.animation_name, vec![name("a")]);
        assert_eq!(m.animation_fill_mode, vec![AnimationFillMode::Both]);
        // An invalid value leaves what came before.
        let m = motion("p { transition: width 1s; transition: none, width 2s }");
        assert_eq!(m.transition_duration, vec![1.0]);
        // A substituted value is the shorthand's; one it does not take is
        // invalid at computed-value time, every longhand `unset`.
        let m = motion("p { --t: opacity 4s; transition: var(--t) }");
        assert_eq!(m.transition_duration, vec![4.0]);
        let m = motion("p { animation: a 1s; --t: 1px; animation: var(--t) }");
        assert_eq!(*ComputedStyle::default().motion(), m);
        // `initial`, `inherit`, `unset` (none is inherited).
        let child = |css: &str| style_of("<div><p>x</p></div>", &[sheet(css)], "p");
        let inherited = child(
            "div { animation: a 1s; transition: opacity 2s } p { animation: inherit; transition-duration: inherit }",
        );
        assert_eq!(inherited.motion().animation_name, vec![name("a")]);
        assert_eq!(inherited.motion().transition_duration, vec![2.0]);
        assert_eq!(
            inherited.motion().transition_property,
            vec![TransitionProperty::All]
        );
        let unset = child("div { animation: a 1s } p { animation: b 2s; animation-name: unset }");
        assert_eq!(unset.motion().animation_name, vec![None]);
        assert_eq!(unset.motion().animation_duration, vec![Some(2.0)]);
        assert!(
            child("div { animation: a 1s } p { color: red }")
                .motion
                .is_none()
        );
        // `@supports` knows them.
        assert!(supports(
            "animation",
            &css::tokenize("spin 1s steps(2, jump-none)")
        ));
        assert!(supports(
            "transition-behavior",
            &css::tokenize("allow-discrete")
        ));
        assert!(!supports("animation-duration", &css::tokenize("-1s")));
    }

    #[test]
    fn a_marker_takes_the_motion_properties_and_a_first_line_does_not() {
        let sheets = [sheet(
            "li::marker { animation: m 1s; transition: color 2s } \
             p::first-line { animation: f 1s; color: red }",
        )];
        let marker =
            pseudo_of("<ol><li>a</li></ol>", &sheets, "li", PseudoElement::Marker).expect("styled");
        assert_eq!(marker.motion().animation_name, vec![Some(Arc::from("m"))]);
        assert_eq!(marker.motion().transition_duration, vec![2.0]);
        let line = pseudo_of("<p>a</p>", &sheets, "p", PseudoElement::FirstLine).expect("styled");
        assert!(line.motion.is_none());
    }

    #[test]
    fn the_flex_shorthand_expands_as_the_tests_and_browsers_have_it() {
        let flex = |value: &str| {
            let s = style_of(
                "<p>x</p>",
                &[sheet(&format!(
                    "p {{ flex-grow: 9; flex-shrink: 9; flex-basis: 9px; flex: {value} }}"
                ))],
                "p",
            );
            (s.flex_grow, s.flex_shrink, s.flex_basis)
        };
        let auto = FlexBasis::Size(Size::Auto);
        let zero_percent = FlexBasis::Size(Size::Percent(0.0));
        let px = |v| FlexBasis::Size(Size::Px(v));
        for (value, expected) in [
            ("none", (0.0, 0.0, auto.clone())),
            ("auto", (1.0, 1.0, auto.clone())),
            ("initial", (0.0, 1.0, auto.clone())),
            // The basis left out is 0% (web-platform-tests'
            // flex-computed.html), the factors 1.
            ("1", (1.0, 1.0, zero_percent.clone())),
            ("2 3", (2.0, 3.0, zero_percent.clone())),
            ("4 5 6px", (4.0, 5.0, px(6.0))),
            ("7% 8", (8.0, 1.0, FlexBasis::Size(Size::Percent(7.0)))),
            ("8 auto", (8.0, 1.0, auto.clone())),
            ("10px 1 2", (1.0, 2.0, px(10.0))),
            ("content", (1.0, 1.0, FlexBasis::Content)),
            // A unitless zero is a factor unless two came before it.
            ("0", (0.0, 1.0, zero_percent.clone())),
            ("1 0", (1.0, 0.0, zero_percent.clone())),
            ("0 0 0", (0.0, 0.0, px(0.0))),
        ] {
            assert_eq!(flex(value), expected, "{value}");
        }
        // Invalid: the declaration is dropped and the longhands stay.
        for value in [
            "1 10px 2",
            "1 2 3",
            "-1",
            "1 -1",
            "1 2 3px 4",
            "10px 20px",
            "1 none",
            "",
        ] {
            assert_eq!(flex(value), (9.0, 9.0, px(9.0)), "{value}");
        }
    }

    #[test]
    fn the_alignment_properties_take_their_grammars() {
        use AlignPosition::*;
        let p = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p");
        let pos = |p, o| ContentAlignment::Position(p, o);
        for (value, expected) in [
            ("normal", ContentAlignment::Normal),
            ("space-between", ContentAlignment::SpaceBetween),
            ("space-around", ContentAlignment::SpaceAround),
            ("space-evenly", ContentAlignment::SpaceEvenly),
            ("stretch", ContentAlignment::Stretch),
            ("center", pos(Center, OverflowAlign::Default)),
            ("flex-end", pos(FlexEnd, OverflowAlign::Default)),
            ("safe end", pos(End, OverflowAlign::Safe)),
            ("unsafe left", pos(Left, OverflowAlign::Unsafe)),
            ("Right", pos(Right, OverflowAlign::Default)),
        ] {
            let s = p(&format!("p {{ justify-content: {value} }}"));
            assert_eq!(s.justify_content, expected, "{value}");
        }
        // justify-content takes no baseline and no self position.
        for value in [
            "baseline",
            "self-start",
            "safe",
            "safe normal",
            "center center",
        ] {
            let s = p(&format!("p {{ justify-content: {value} }}"));
            assert_eq!(s.justify_content, ContentAlignment::Normal, "{value}");
        }
        for (value, expected) in [
            (
                "baseline",
                ContentAlignment::Baseline(BaselinePosition::First),
            ),
            (
                "first baseline",
                ContentAlignment::Baseline(BaselinePosition::First),
            ),
            (
                "last baseline",
                ContentAlignment::Baseline(BaselinePosition::Last),
            ),
            ("space-evenly", ContentAlignment::SpaceEvenly),
            ("safe flex-start", pos(FlexStart, OverflowAlign::Safe)),
        ] {
            let s = p(&format!("p {{ align-content: {value} }}"));
            assert_eq!(s.align_content, expected, "{value}");
        }
        for value in ["left", "baseline first", "last", "self-end"] {
            let s = p(&format!("p {{ align-content: {value} }}"));
            assert_eq!(s.align_content, ContentAlignment::Normal, "{value}");
        }
        let sp = |p, o| SelfAlignment::Position(p, o);
        for (value, expected) in [
            ("stretch", SelfAlignment::Stretch),
            ("baseline", SelfAlignment::Baseline(BaselinePosition::First)),
            (
                "last baseline",
                SelfAlignment::Baseline(BaselinePosition::Last),
            ),
            ("self-end", sp(SelfEnd, OverflowAlign::Default)),
            ("unsafe center", sp(Center, OverflowAlign::Unsafe)),
        ] {
            let s = p(&format!(
                "p {{ align-items: {value}; align-self: {value} }}"
            ));
            assert_eq!(
                (s.align_items, s.align_self),
                (expected, expected),
                "{value}"
            );
        }
        // `auto` is align-self's alone; neither takes left or right.
        let s = p("p { align-items: auto; align-self: auto }");
        assert_eq!(
            (s.align_items, s.align_self),
            (SelfAlignment::Normal, SelfAlignment::Auto)
        );
        let s = p("p { align-items: left; align-self: right }");
        assert_eq!(
            (s.align_items, s.align_self),
            (SelfAlignment::Normal, SelfAlignment::Auto)
        );
    }

    #[test]
    fn the_place_shorthands_set_the_halves_this_engine_has() {
        use AlignPosition::*;
        let p = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p");
        let pos = |p| ContentAlignment::Position(p, OverflowAlign::Default);
        let s = p("p { place-content: center space-between }");
        assert_eq!(
            (s.align_content, s.justify_content),
            (pos(Center), ContentAlignment::SpaceBetween)
        );
        // One value is both, but a baseline gives `start` to the second.
        let s = p("p { place-content: end }");
        assert_eq!((s.align_content, s.justify_content), (pos(End), pos(End)));
        let s = p("p { place-content: last baseline }");
        assert_eq!(
            (s.align_content, s.justify_content),
            (
                ContentAlignment::Baseline(BaselinePosition::Last),
                pos(Start)
            )
        );
        // Two-word halves on either side.
        let s = p("p { place-content: safe center unsafe right }");
        assert_eq!(
            (s.align_content, s.justify_content),
            (
                ContentAlignment::Position(Center, OverflowAlign::Safe),
                ContentAlignment::Position(Right, OverflowAlign::Unsafe)
            )
        );
        // `left` is no align-content: the whole declaration is invalid.
        let s = p("p { place-content: left center }");
        assert_eq!(
            (s.align_content, s.justify_content),
            (ContentAlignment::Normal, ContentAlignment::Normal)
        );
        // Without a second value, the justify half is the first (M18).
        let s = p("p { place-items: center; place-self: end }");
        assert_eq!(
            (s.align_items, s.align_self),
            (
                SelfAlignment::Position(Center, OverflowAlign::Default),
                SelfAlignment::Position(End, OverflowAlign::Default)
            )
        );
        assert_eq!(
            (s.grid().justify_items.value, s.grid().justify_self),
            (
                SelfAlignment::Position(Center, OverflowAlign::Default),
                SelfAlignment::Position(End, OverflowAlign::Default)
            )
        );
        // The justify half is its own property's: `legacy` is
        // justify-items', `auto` and `left` justify-self's.
        let s = p("p { place-items: start legacy left; place-self: stretch auto }");
        assert_eq!(
            (s.align_items, s.align_self),
            (
                SelfAlignment::Position(Start, OverflowAlign::Default),
                SelfAlignment::Stretch
            )
        );
        assert_eq!(
            s.grid().justify_items,
            JustifyItems {
                value: SelfAlignment::Position(Left, OverflowAlign::Default),
                legacy: true
            }
        );
        assert_eq!(s.grid().justify_self, SelfAlignment::Auto);
        let s = p("p { place-self: auto safe right }");
        assert_eq!(
            (s.align_self, s.grid().justify_self),
            (
                SelfAlignment::Auto,
                SelfAlignment::Position(Right, OverflowAlign::Safe)
            )
        );
        let s = p("p { place-items: start auto; place-self: stretch legacy }");
        assert_eq!(
            (s.align_items, s.align_self),
            (SelfAlignment::Normal, SelfAlignment::Auto)
        );
        assert_eq!(s.grid, None);
    }

    #[test]
    fn gaps_are_normal_or_lengths() {
        let p = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p");
        let s = p("p { row-gap: 10px; column-gap: 5% }");
        assert_eq!(
            (s.row_gap, s.column_gap),
            (
                Some(LengthPercentage::Px(10.0)),
                Some(LengthPercentage::Percent(5.0))
            )
        );
        let s = p("p { gap: 1em }");
        assert_eq!(
            (s.row_gap, s.column_gap),
            (
                Some(LengthPercentage::Px(16.0)),
                Some(LengthPercentage::Px(16.0))
            )
        );
        let s = p("p { gap: 2px normal }");
        assert_eq!(
            (s.row_gap, s.column_gap),
            (Some(LengthPercentage::Px(2.0)), None)
        );
        // Negative, three values: invalid; css-wide keywords reset both.
        for css in [
            "p { gap: 3px; gap: -1px }",
            "p { gap: 3px; gap: 1px 2px 3px }",
            "p { row-gap: 3px; column-gap: 3px; row-gap: -2px; column-gap: auto }",
        ] {
            let s = p(css);
            assert_eq!(
                (s.row_gap, s.column_gap),
                (
                    Some(LengthPercentage::Px(3.0)),
                    Some(LengthPercentage::Px(3.0))
                ),
                "{css}"
            );
        }
        let s = p("p { gap: 3px; gap: initial }");
        assert_eq!((s.row_gap, s.column_gap), (None, None));
    }

    #[test]
    fn sizes_take_the_intrinsic_keywords_and_no_negative_value() {
        let p = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p");
        let s = p(
            "p { width: min-content; height: MAX-CONTENT; min-width: fit-content;
                       max-width: max-content; max-height: min-content; min-height: auto }",
        );
        assert_eq!(
            (s.width, s.height, s.min_width, s.min_height),
            (
                Size::MinContent,
                Size::MaxContent,
                Size::FitContent,
                Size::Auto
            )
        );
        assert_eq!(
            (s.max_width, s.max_height),
            (MaxSize::MaxContent, MaxSize::MinContent)
        );
        // `auto` is no maximum, `none` no size; negative sizes are invalid.
        let s = p(
            "p { width: 5px; width: -5px; max-width: 5px; max-width: auto;
                       min-height: 5px; min-height: none; height: 5%; height: -5% }",
        );
        assert_eq!(
            (s.width, s.max_width, s.min_height, s.height),
            (
                Size::Px(5.0),
                MaxSize::Px(5.0),
                Size::Px(5.0),
                Size::Percent(5.0)
            )
        );
    }

    #[test]
    fn the_webkit_aliases_are_the_properties_under_a_second_name() {
        let p = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p");
        let s = p(
            "p { -webkit-flex: 2; -webkit-flex-flow: column wrap; -webkit-order: 3;
                       -webkit-justify-content: center; -webkit-align-items: end;
                       -webkit-align-self: start; -webkit-align-content: stretch;
                       -webkit-box-sizing: border-box }",
        );
        assert_eq!(
            (s.flex_grow, s.flex_shrink, s.flex_basis),
            (2.0, 1.0, FlexBasis::Size(Size::Percent(0.0)))
        );
        assert_eq!(
            (s.flex_direction, s.flex_wrap, s.order),
            (FlexDirection::Column, FlexWrap::Wrap, 3)
        );
        assert_eq!(
            s.justify_content,
            ContentAlignment::Position(AlignPosition::Center, OverflowAlign::Default)
        );
        assert_eq!(s.align_content, ContentAlignment::Stretch);
        assert_eq!(s.box_sizing, BoxSizing::BorderBox);
        let s = p(
            "p { -webkit-flex-grow: 4; -webkit-flex-shrink: 5; -webkit-flex-basis: 6px;
                       -webkit-flex-direction: row-reverse; -webkit-flex-wrap: wrap-reverse }",
        );
        assert_eq!(
            (s.flex_grow, s.flex_shrink, s.flex_basis),
            (4.0, 5.0, FlexBasis::Size(Size::Px(6.0)))
        );
        assert_eq!(
            (s.flex_direction, s.flex_wrap),
            (FlexDirection::RowReverse, FlexWrap::WrapReverse)
        );
        // An alias competes as the property: the later declaration wins,
        // whichever name it uses, and css-wide keywords work through it.
        let s = p("p { flex-grow: 1; -webkit-flex-grow: 2 }");
        assert_eq!(s.flex_grow, 2.0);
        let s = p("p { -webkit-flex-grow: 2; flex-grow: 1 }");
        assert_eq!(s.flex_grow, 1.0);
        let s = p("p { flex: 3; -webkit-flex: initial }");
        assert_eq!(s.flex_grow, 0.0);
        // Not an alias the Standard lists.
        assert_eq!(p("p { -webkit-gap: 5px }").row_gap, None);
    }

    #[test]
    fn css_wide_keywords_reach_the_flex_shorthands() {
        let html = "<div><p>x</p></div>";
        let s = style_of(
            html,
            &[sheet(
                "div { flex: 2 3 4px; flex-flow: column wrap; gap: 5px 6px;
                       place-content: center end; place-items: end; place-self: start }
                 p { flex: inherit; flex-flow: inherit; gap: inherit;
                     place-content: inherit; place-items: inherit; place-self: inherit }",
            )],
            "p",
        );
        let parent = style_of(
            html,
            &[sheet(
                "div { flex: 2 3 4px; flex-flow: column wrap; gap: 5px 6px;
                       place-content: center end; place-items: end; place-self: start }",
            )],
            "div",
        );
        assert_eq!(
            (s.flex_grow, s.flex_shrink, s.flex_basis),
            (parent.flex_grow, parent.flex_shrink, parent.flex_basis)
        );
        assert_eq!(
            (s.flex_direction, s.flex_wrap),
            (parent.flex_direction, parent.flex_wrap)
        );
        assert_eq!(
            (s.row_gap, s.column_gap),
            (parent.row_gap, parent.column_gap)
        );
        assert_eq!(
            (
                s.align_content,
                s.justify_content,
                s.align_items,
                s.align_self
            ),
            (
                parent.align_content,
                parent.justify_content,
                parent.align_items,
                parent.align_self
            )
        );
        // `unset` on a property that is not inherited is `initial`.
        let s = style_of(
            html,
            &[sheet("div { order: 4 } p { order: 2; order: unset }")],
            "p",
        );
        assert_eq!(s.order, 0);
    }

    #[test]
    fn outline_and_cursor_compute_as_css_ui_says() {
        use crate::computed::{Cursor, OutlineStyle};
        let html = "<div><p>x</p></div>";
        let pair = |css: &str| {
            let sheets = [sheet(css)];
            (style_of(html, &sheets, "div"), style_of(html, &sheets, "p"))
        };
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        // The outline is not inherited; the cursor is, and its images'
        // fallback keyword is what it is.
        let (div, p) = pair(
            "div { outline: 2px dashed red; outline-offset: -1px; \
             cursor: url(a.png) 1 2, url(b.cur), help; color: blue }",
        );
        assert_eq!(div.outline().style, OutlineStyle::Dashed);
        assert_eq!(div.outline().width, 2.0);
        assert_eq!(div.outline_color(), red);
        assert_eq!(div.outline().offset, -1.0);
        assert_eq!((div.cursor, p.cursor), (Cursor::Help, Cursor::Help));
        assert_eq!(p.outline().style, OutlineStyle::None);
        assert_eq!(p.used_outline_width(), 0.0);
        // A colour left out is `currentcolor`; a width left out medium;
        // the shorthand resets what it leaves out.
        let (div, _) = pair("div { outline-color: red; outline: solid; color: blue }");
        assert_eq!((div.outline_color(), div.used_outline_width()), (blue, 3.0));
        let (div, _) = pair("div { outline: thick solid; color: #00f }");
        assert_eq!(div.used_outline_width(), 5.0);
        let (div, _) = pair("div { outline-style: auto; outline-color: -webkit-focus-ring-color }");
        assert_eq!(div.outline().style, OutlineStyle::Auto);
        assert_eq!(div.outline_color(), Color::rgb8(16, 16, 16));
        // What does not parse is dropped.
        for bad in [
            "cursor: url(a.png)",
            "cursor: hand",
            "cursor: help, pointer",
            "outline: red blue",
            "outline-style: hidden",
            "outline-width: -1px",
        ] {
            let (div, _) = pair(&format!(
                "div {{ cursor: move; outline: 1px dotted red; {bad} }}"
            ));
            assert_eq!(div.cursor, Cursor::Move, "{bad}");
            assert_eq!(div.outline().style, OutlineStyle::Dotted, "{bad}");
            assert_eq!(div.used_outline_width(), 1.0, "{bad}");
        }
        // As a script reads them.
        let (div, _) = pair("div { outline: 2.7px groove; color: #ff0000; cursor: NOT-allowed }");
        let value = |name| crate::serialize::computed_value(&div, name);
        assert_eq!(value("outline-style").as_deref(), Some("groove"));
        assert_eq!(value("outline-width").as_deref(), Some("2px"));
        assert_eq!(value("outline-color").as_deref(), Some("rgb(255, 0, 0)"));
        assert_eq!(value("outline-offset").as_deref(), Some("0px"));
        assert_eq!(value("cursor").as_deref(), Some("not-allowed"));
    }

    #[test]
    fn table_properties_parse_and_inherit_as_the_specification_says() {
        let html = "<div><p>x</p></div>";
        let pair = |css: &str| {
            let sheets = [sheet(css)];
            (style_of(html, &sheets, "div"), style_of(html, &sheets, "p"))
        };
        // `scroll-behavior`: not inherited, `auto` or `smooth`.
        let (div, p) = pair("div { scroll-behavior: Smooth; }");
        assert!(div.effects().scroll_behavior_smooth);
        assert!(!p.effects().scroll_behavior_smooth);
        let (div, _) = pair("div { scroll-behavior: smooth; scroll-behavior: instant; }");
        assert!(div.effects().scroll_behavior_smooth, "instant is no value");
        assert_eq!(
            crate::serialize::computed_value(&div, "scroll-behavior").as_deref(),
            Some("smooth")
        );
        // `table-layout`: not inherited.
        let (div, p) = pair("div { table-layout: FIXED; }");
        assert_eq!(div.table_layout, TableLayout::Fixed);
        assert_eq!(p.table_layout, TableLayout::Auto);
        let (div, _) = pair("div { table-layout: fixed; table-layout: auto; }");
        assert_eq!(div.table_layout, TableLayout::Auto);
        let (div, _) = pair("div { table-layout: fixed; table-layout: tight; }");
        assert_eq!(
            div.table_layout,
            TableLayout::Fixed,
            "an unknown value is ignored"
        );
        // `border-collapse`: inherited.
        let (div, p) = pair("div { border-collapse: collapse; }");
        assert_eq!(div.border_collapse, BorderCollapse::Collapse);
        assert_eq!(p.border_collapse, BorderCollapse::Collapse);
        let (_, p) = pair("div { border-collapse: collapse; } p { border-collapse: separate; }");
        assert_eq!(p.border_collapse, BorderCollapse::Separate);
        // `border-spacing`: one length for both, or horizontal then
        // vertical; `em` of the element's own font size; never negative,
        // never a percentage; inherited.
        let spacing = |h: f32, v: f32| BorderSpacing {
            horizontal: h,
            vertical: v,
        };
        let (div, p) = pair("div { border-spacing: 3px; }");
        assert_eq!(div.border_spacing, spacing(3.0, 3.0));
        assert_eq!(p.border_spacing, spacing(3.0, 3.0));
        let (div, _) = pair("div { border-spacing: 4px 0; }");
        assert_eq!(div.border_spacing, spacing(4.0, 0.0));
        let (div, _) = pair("div { font-size: 10px; border-spacing: 1em 0.5em; }");
        assert_eq!(div.border_spacing, spacing(10.0, 5.0));
        for bad in ["-1px", "2px -1px", "10%", "1px 2px 3px", "auto"] {
            let (div, _) = pair(&format!(
                "div {{ border-spacing: 3px; border-spacing: {bad}; }}"
            ));
            assert_eq!(div.border_spacing, spacing(3.0, 3.0), "{bad} is ignored");
        }
        // `caption-side`: `top` or `bottom`; inherited.
        let (div, p) = pair("div { caption-side: bottom; }");
        assert_eq!(div.caption_side, CaptionSide::Bottom);
        assert_eq!(p.caption_side, CaptionSide::Bottom);
        let (div, _) = pair("div { caption-side: bottom; caption-side: left; }");
        assert_eq!(
            div.caption_side,
            CaptionSide::Bottom,
            "CSS2's `left` is not a value"
        );
        // `empty-cells`: inherited.
        let (div, p) = pair("div { empty-cells: hide; }");
        assert_eq!(div.empty_cells, EmptyCells::Hide);
        assert_eq!(p.empty_cells, EmptyCells::Hide);
        let (_, p) = pair("div { empty-cells: hide; } p { empty-cells: show; }");
        assert_eq!(p.empty_cells, EmptyCells::Show);
        // `vertical-align`: every keyword, a length, a percentage; not
        // inherited.
        for (value, expected) in [
            ("baseline", VerticalAlign::Baseline),
            ("sub", VerticalAlign::Sub),
            ("super", VerticalAlign::Super),
            ("top", VerticalAlign::Top),
            ("text-top", VerticalAlign::TextTop),
            ("MIDDLE", VerticalAlign::Middle),
            ("bottom", VerticalAlign::Bottom),
            ("text-bottom", VerticalAlign::TextBottom),
            ("3px", VerticalAlign::Length(3.0)),
            ("-0.5em", VerticalAlign::Length(-8.0)),
            ("0", VerticalAlign::Length(0.0)),
            ("25%", VerticalAlign::Percent(25.0)),
            ("-10%", VerticalAlign::Percent(-10.0)),
        ] {
            let (div, p) = pair(&format!("div {{ vertical-align: {value}; }}"));
            assert_eq!(div.vertical_align, expected, "{value}");
            assert_eq!(
                p.vertical_align,
                VerticalAlign::Baseline,
                "not inherited ({value})"
            );
        }
        let (div, _) = pair("div { vertical-align: top; vertical-align: center; }");
        assert_eq!(
            div.vertical_align,
            VerticalAlign::Top,
            "`center` is not a value"
        );
        // The CSS-wide keywords: `unset` is `inherit` for the four
        // inherited properties and `initial` for the other two.
        let (_, p) = pair(
            "div { table-layout: fixed; border-collapse: collapse; border-spacing: 5px;
                   caption-side: bottom; empty-cells: hide; vertical-align: middle; }
             p { table-layout: unset; border-collapse: unset; border-spacing: unset;
                 caption-side: unset; empty-cells: unset; vertical-align: unset; }",
        );
        assert_eq!(p.table_layout, TableLayout::Auto);
        assert_eq!(p.border_collapse, BorderCollapse::Collapse);
        assert_eq!(p.border_spacing, spacing(5.0, 5.0));
        assert_eq!(p.caption_side, CaptionSide::Bottom);
        assert_eq!(p.empty_cells, EmptyCells::Hide);
        assert_eq!(p.vertical_align, VerticalAlign::Baseline);
        let (_, p) = pair(
            "div { table-layout: fixed; border-collapse: collapse; border-spacing: 5px;
                   caption-side: bottom; empty-cells: hide; vertical-align: middle; }
             p { table-layout: inherit; border-collapse: initial; border-spacing: initial;
                 caption-side: initial; empty-cells: initial; vertical-align: inherit; }",
        );
        assert_eq!(p.table_layout, TableLayout::Fixed);
        assert_eq!(p.border_collapse, BorderCollapse::Separate);
        assert_eq!(p.border_spacing, spacing(0.0, 0.0));
        assert_eq!(p.caption_side, CaptionSide::Top);
        assert_eq!(p.empty_cells, EmptyCells::Show);
        assert_eq!(p.vertical_align, VerticalAlign::Middle);
    }

    #[test]
    fn the_ua_sheet_styles_tables_as_the_html_standard_does() {
        let html = "<table><caption>c</caption><colgroup><col></colgroup>\
                    <thead><tr><th>h</th></tr></thead>\
                    <tbody><tr><td>d</td></tr></tbody>\
                    <tfoot><tr><td>f</td></tr></tfoot></table>\
                    <table><tr><td>bare</td></tr></table>\
                    <div><div class=cell>css</div></div>";
        let sheets = [sheet(
            "table { border-color: red; } .cell { display: table-cell; }",
        )];
        let of = |selector: &str| style_of(html, &sheets, selector);
        for (selector, expected) in [
            ("table", Display::Table),
            ("caption", Display::TableCaption),
            ("colgroup", Display::TableColumnGroup),
            ("col", Display::TableColumn),
            ("thead", Display::TableHeaderGroup),
            ("tbody", Display::TableRowGroup),
            ("tfoot", Display::TableFooterGroup),
            ("tr", Display::TableRow),
            ("th", Display::TableCell),
            ("td", Display::TableCell),
        ] {
            assert_eq!(of(selector).display, expected, "{selector}");
        }
        let table = of("table");
        assert_eq!(table.box_sizing, BoxSizing::BorderBox);
        assert_eq!(table.border_collapse, BorderCollapse::Separate);
        assert_eq!(
            table.border_spacing,
            BorderSpacing {
                horizontal: 2.0,
                vertical: 2.0
            }
        );
        assert_eq!(table.table_layout, TableLayout::Auto);
        let td = of("tbody td");
        assert_eq!(td.padding, [const { LengthPercentage::Px(1.0) }; 4]);
        assert_eq!(td.text_align, TextAlign::Start);
        assert_eq!(of("th").text_align, TextAlign::Center);
        assert_eq!(of("th").padding, td.padding);
        assert_eq!(of("caption").text_align, TextAlign::Center);
        // An HTML cell is centred vertically: the row group says
        // `middle`, and `tr`, `td` and `th` inherit it, the `tbody` the
        // parser makes for a bare row included; a CSS table cell sits on
        // the baseline.
        for selector in [
            "thead", "tbody", "tfoot", "thead tr", "th", "tbody td", "tfoot td",
        ] {
            assert_eq!(
                of(selector).vertical_align,
                VerticalAlign::Middle,
                "{selector}"
            );
        }
        assert_eq!(of("table + table tr").vertical_align, VerticalAlign::Middle);
        assert_eq!(of("table + table td").vertical_align, VerticalAlign::Middle);
        assert_eq!(of(".cell").vertical_align, VerticalAlign::Baseline);
        assert_eq!(
            of(".cell").padding,
            [const { LengthPercentage::Px(0.0) }; 4]
        );
        // Row groups and rows take the table's border colour; a cell
        // keeps `currentcolor`.
        let red = Color::rgb8(255, 0, 0);
        assert_eq!(of("tbody").border_color, [red; 4]);
        assert_eq!(of("tbody tr").border_color, [red; 4]);
        assert_eq!(td.border_color, [Color::CURRENT; 4]);
        // The caption and the column boxes have no padding or border of
        // their own from the sheet.
        assert_eq!(
            of("caption").padding,
            [const { LengthPercentage::Px(0.0) }; 4]
        );
        assert_eq!(of("col").used_border_width(), [0.0; 4]);
    }

    #[test]
    fn the_ua_sheet_styles_form_controls_as_boxes() {
        // M26.7: the reference browser's computed values. A text field
        // at 13.33px inside a 2px inset border, a textarea in monospace
        // and a scroll container, a select, all their sizes `auto`, their
        // own the box tree's (M26.11); options and their groups blocks
        // (which a drop-down does not lay out), a datalist and a hidden
        // input nothing, a checkbox no border, its 13px the mark's natural
        // size.
        let html = "<form><input><input type=HIDDEN><input type=checkbox>\
                    <button>Go</button><select><optgroup><option>a</option></optgroup></select>\
                    <datalist></datalist><textarea>t</textarea></form>";
        let control = |selector: &str| style_of(html, &[], selector);
        let input = control("input:first-child");
        assert_eq!(input.display, Display::InlineBlock);
        assert_eq!(input.font_size_px, 13.3333);
        assert_eq!(
            (input.width.clone(), input.height.clone()),
            (Size::Auto, Size::Auto)
        );
        assert_eq!(input.used_border_width(), [2.0; 4]);
        assert_eq!(input.border_style[TOP], BorderStyle::Inset);
        assert_eq!(input.padding[TOP], LengthPercentage::Px(1.0));
        assert_eq!(input.padding[LEFT], LengthPercentage::Px(2.0));
        assert_eq!(input.background_color, Color::rgb8(255, 255, 255));
        assert_eq!(
            control("input[type=HIDDEN]").display,
            Display::None,
            "the UA rule matches the type whatever its case"
        );
        let checkbox = control("input[type=checkbox]");
        assert_eq!(checkbox.width, Size::Auto, "the mark's natural size");
        assert_eq!(checkbox.box_sizing, BoxSizing::BorderBox);
        assert_eq!(checkbox.used_border_width(), [0.0; 4]);
        let button = control("button");
        assert_eq!(button.width, Size::Auto);
        assert_eq!(button.text_align, TextAlign::Center);
        assert_eq!(button.background_color, Color::rgb8(240, 240, 240));
        let select = control("select");
        assert_eq!(
            (select.width.clone(), select.height.clone()),
            (Size::Auto, Size::Auto)
        );
        let option = control("option");
        assert_eq!(
            option.display,
            Display::Block,
            "a block, as in the reference browser"
        );
        assert_eq!(option.padding[LEFT], LengthPercentage::Px(2.0));
        assert_eq!(control("datalist").display, Display::None);
        assert_eq!(control("optgroup").display, Display::Block);
        let textarea = control("textarea");
        assert_eq!(
            (textarea.width.clone(), textarea.height.clone()),
            (Size::Auto, Size::Auto)
        );
        assert_eq!(&*textarea.font().family, "monospace");
        assert_eq!(textarea.white_space, WhiteSpace::PRE_WRAP);
        assert_eq!(textarea.overflow(), Overflow::Auto);
    }

    #[test]
    fn hex_colors_all_four_lengths() {
        assert_eq!(
            parse_color(&css::tokenize("#f00")),
            Some(Color::rgb8(255, 0, 0))
        );
        assert_eq!(
            parse_color(&css::tokenize("#ff0000")),
            Some(Color::rgb8(255, 0, 0))
        );
        assert_eq!(
            parse_color(&css::tokenize("#f00f")),
            Some(Color::rgba(1.0, 0.0, 0.0, 1.0))
        );
        assert_eq!(
            parse_color(&css::tokenize("#ff000080")),
            Some(Color::rgba(1.0, 0.0, 0.0, 128.0 / 255.0))
        );
        assert_eq!(parse_color(&css::tokenize("#ggg")), None);
        assert_eq!(parse_color(&css::tokenize("#ff")), None); // invalid length
    }

    #[test]
    fn rgb_and_rgba_are_read_in_both_syntaxes() {
        let color = |text: &str| parse_color(&css::tokenize(text));
        let half = Color {
            a: 0.5,
            ..Color::rgb8(255, 0, 51)
        };
        for text in [
            "rgb(255, 0, 51)",
            "RGB(255,0,51)",
            "rgba(255, 0, 51)",
            "rgb(100%, 0%, 20%)",
            "rgb(255 0 51)",
            "rgb(100% 0 51)",
            "rgb(255 none 51)",
            "rgb(300, -5, 51)",
        ] {
            assert_eq!(color(text), Some(Color::rgb8(255, 0, 51)), "{text}");
        }
        for text in [
            "rgba(255, 0, 51, 0.5)",
            "rgb(255, 0, 51, 50%)",
            "rgb(255 0 51 / 0.5)",
            "rgba(255 0 51 / 50%)",
        ] {
            assert_eq!(color(text), Some(half), "{text}");
        }
        assert_eq!(color("rgb(0 0 0 / 2)").map(|c| c.a), Some(1.0));
        for text in [
            "rgb(255, 0%, 51)",
            "rgb(255, none, 51)",
            "rgb(255, 0, 51, none)",
            "rgb(255, 0)",
            "rgb(255 0 51 0.5)",
            "rgb(255 0 51 x 0.5)",
            "rgb(255, 0, 51, 1, 1)",
            "rgb(255 0 51 / 0.5 / 1)",
            "rgb(1px 0 0)",
            "rgb(255 0 51",
            "lab(0% 0 0 0)",
        ] {
            assert_eq!(color(text), None, "{text}");
        }
    }

    #[test]
    fn hsl_and_hwb_are_read_as_css_color_4_converts_them() {
        let color = |text: &str| parse_color(&css::tokenize(text));
        let rgb8 = |text: &str| {
            color(text).map(|c| [c.r, c.g, c.b, c.a].map(|v| (v * 255.0).round() as u8))
        };
        for (text, want) in [
            ("hsl(120, 100%, 50%)", [0, 255, 0, 255]),
            ("hsl(120 100% 50%)", [0, 255, 0, 255]),
            ("hsla(0, 100%, 50%, 0.5)", [255, 0, 0, 128]),
            ("hsl(240deg 100 50 / 50%)", [0, 0, 255, 128]),
            ("hsl(0.5turn 100% 25%)", [0, 128, 128, 255]),
            ("hsl(200grad 100% 50%)", [0, 255, 255, 255]),
            ("hsl(-120 100% 50%)", [0, 0, 255, 255]),
            ("hsl(30 50% 60%)", [204, 153, 102, 255]),
            ("hsl(none 0% 100%)", [255, 255, 255, 255]),
            ("hwb(0 0% 0%)", [255, 0, 0, 255]),
            ("hwb(120 20% 30%)", [51, 179, 51, 255]),
            ("hwb(0 60% 60%)", [128, 128, 128, 255]),
        ] {
            assert_eq!(rgb8(text), Some(want), "{text}");
        }
        for text in [
            "hsl(120, 100, 50%)",
            "hsl(120, none, 50%)",
            "hwb(0, 0%, 0%)",
            "hsl(120 100% 50% 1)",
            "hsl(1px 100% 50%)",
            "hsl(120 100%)",
        ] {
            assert_eq!(color(text), None, "{text}");
        }
        assert_eq!(parse_color_text("hsl(120 100% 50%)"), color("#0f0"));
        // A function left open at the end is closed there.
        assert_eq!(parse_color_text("rgb(0, 255, 0"), color("#0f0"));
        assert_eq!(parse_color_text("rgba(0, 255, 0, 1"), color("#0f0"));
        assert_eq!(parse_color_text("rgb(0, 255"), None);
    }

    #[test]
    fn named_colors_are_case_insensitive() {
        assert_eq!(
            parse_color(&css::tokenize("RED")),
            Some(Color::rgb8(255, 0, 0))
        );
        assert_eq!(
            parse_color(&css::tokenize("CornflowerBlue")),
            Some(Color::rgb8(100, 149, 237))
        );
        assert_eq!(parse_color(&css::tokenize("cornflowerblu")), None);
    }

    #[test]
    fn the_named_colours_are_css_color_4s_whole_table() {
        // Sorted and distinct, so the binary search finds every one.
        assert!(NAMED_COLORS.windows(2).all(|w| w[0].0 < w[1].0));
        // Both spellings of every grey, and the ones M5 lacked.
        for (name, rgb) in [
            ("aliceblue", [240, 248, 255]),
            ("darkslategray", [47, 79, 79]),
            ("darkslategrey", [47, 79, 79]),
            ("lightblue", [173, 216, 230]),
            ("lightgreen", [144, 238, 144]),
            ("pink", [255, 192, 203]),
            ("rebeccapurple", [102, 51, 153]),
            ("yellowgreen", [154, 205, 50]),
            ("orange", [255, 165, 0]),
        ] {
            let [r, g, b] = rgb;
            assert_eq!(named_color(name), Some(Color::rgb8(r, g, b)), "{name}");
        }
        assert_eq!(named_color("Transparent"), Some(Color::TRANSPARENT));
        assert_eq!(named_color("currentcolor"), None);
    }

    #[test]
    fn font_family_list_preserves_multi_word_and_quoted_names() {
        assert_eq!(
            parse_font_family(&css::tokenize("Arial, sans-serif")),
            Some("Arial, sans-serif".to_string())
        );
        assert_eq!(
            parse_font_family(&css::tokenize("\"Segoe UI\", sans-serif")),
            Some("Segoe UI, sans-serif".to_string())
        );
        assert_eq!(
            parse_font_family(&css::tokenize("Segoe UI, sans-serif")),
            Some("Segoe UI, sans-serif".to_string())
        );
    }

    #[test]
    fn zero_length_needs_no_unit_but_nonzero_does() {
        assert_eq!(
            with_bases(|b| parse_length_px(&css::tokenize("0"), b)),
            Some(0.0)
        );
        assert_eq!(
            with_bases(|b| parse_length_px(&css::tokenize("5"), b)),
            None
        );
        assert_eq!(
            with_bases(|b| parse_length_px(&css::tokenize("5px"), b)),
            Some(5.0)
        );
    }

    #[test]
    fn deleting_the_style_block_falls_back_to_ua_styling() {
        // The M5.9 acceptance scenario, at the unit level: no author
        // sheets at all still yields real (UA) styling, not blank defaults.
        let with_author = style_of("<h1>x</h1>", &[sheet("h1 { color: red; }")], "h1");
        let ua_only = style_of("<h1>x</h1>", &[], "h1");
        assert_eq!(with_author.font_size_px, ua_only.font_size_px); // both from ua.css
        assert_ne!(with_author.color, Color::BLACK); // author override present
        assert_eq!(ua_only.display, Display::Block); // ua.css alone still styles it
    }

    #[test]
    fn non_rendering_elements_default_to_display_none() {
        // Without this, `<style>`'s own CSS source text (or `<title>`'s,
        // `<script>`'s, ...) would compute as ordinary visible inline
        // content — a real bug caught at the full-pipeline level (M6)
        // before being traced back to a missing UA rule here.
        for tag in ["head", "style", "script", "title", "meta", "link"] {
            let html = format!("<{tag}>x</{tag}>");
            let s = style_of(&html, &[], tag);
            assert_eq!(
                s.display,
                Display::None,
                "{tag} should default to display:none"
            );
        }
    }

    #[test]
    fn pre_defaults_to_white_space_pre() {
        let s = style_of("<pre>x</pre>", &[], "pre");
        assert_eq!(s.white_space, WhiteSpace::PRE);
        assert_eq!(s.display, Display::Block);
    }

    #[test]
    fn adding_a_color_rule_visibly_changes_computed_style() {
        let before = style_of("<p>x</p>", &[], "p");
        let after = style_of("<p>x</p>", &[sheet("p { color: red; }")], "p");
        assert_ne!(before.color, after.color);
    }

    #[test]
    fn grid_and_inline_grid_make_containers_whose_children_are_blockified() {
        // css-grid-2 §5.1, §6.1 (M18).
        for (value, expected) in [
            ("grid", Display::Grid),
            ("inline-grid", Display::InlineGrid),
            ("GRID", Display::Grid),
        ] {
            let s = style_of(
                "<p>x</p>",
                &[sheet(&format!("p {{ display: {value} }}"))],
                "p",
            );
            assert_eq!(s.display, expected, "{value}");
        }
        let html = "<div><span>a</span><i>i</i><u>u</u><q>q</q></div>";
        for container in ["grid", "inline-grid"] {
            let sheets = [sheet(&format!(
                "div {{ display: {container} }} i {{ display: inline-block }}
                 u {{ display: inline-grid }} q {{ display: none }}"
            ))];
            for (selector, expected) in [
                ("span", Display::Block),
                ("i", Display::Block),
                ("u", Display::Grid),
                ("q", Display::None),
            ] {
                assert_eq!(
                    style_of(html, &sheets, selector).display,
                    expected,
                    "{container} {selector}"
                );
            }
        }
        // §5.1: a floated or absolutely positioned inline grid is a grid.
        for extra in ["float: left", "position: absolute"] {
            let css = format!("p {{ display: inline-grid; {extra} }}");
            let s = style_of("<p>x</p>", &[sheet(&css)], "p");
            assert_eq!(s.display, Display::Grid, "{extra}");
        }
        // The container's `::before` is an item.
        let sheets = [sheet(
            "div { display: grid } div::before { content: 'x'; display: inline }",
        )];
        assert_eq!(
            pseudo_of("<div>x</div>", &sheets, "div", PseudoElement::Before).map(|s| s.display),
            Some(Display::Block)
        );
    }

    /// The grid properties `css` gives a `<p>`.
    fn grid_of(css: &str) -> crate::computed::GridStyle {
        style_of("<p>x</p>", &[sheet(css)], "p").grid().clone()
    }

    #[test]
    fn math_functions_compute_where_their_values_do() {
        use crate::computed::{TransformFunction, Translate};
        // M21.2: a math function is computed with the element's bases: a
        // value with no percentage left is a length, a number or an
        // integer, fitted to its property's range rather than dropped; one
        // with a percentage keeps its expression for layout.
        let of = |css: &str| {
            let sheets = [sheet(&format!(
                "div {{ font-size: 20px }} p {{ width: 7px; {css} }}"
            ))];
            style_of("<div><p>x</p></div>", &sheets, "p")
        };
        assert_eq!(of("width: calc(10px + 2em)").width, Size::Px(10.0 + 40.0));
        assert_eq!(of("width: calc(10px - 20px)").width, Size::Px(0.0));
        assert_eq!(of("width: calc(1px + 1)").width, Size::Px(7.0), "invalid");
        let Size::Calc(width) = of("width: calc(100% - 2em)").width else {
            panic!("an expression");
        };
        assert_eq!(width.resolve(200.0), 200.0 - 40.0);
        assert_eq!(width.resolve(10.0), 0.0, "never negative");
        assert_eq!(
            of("margin: calc(-10px) calc(1px + 2px)").margin,
            [
                LengthPercentageAuto::Px(-10.0),
                LengthPercentageAuto::Px(3.0),
                LengthPercentageAuto::Px(-10.0),
                LengthPercentageAuto::Px(3.0),
            ]
        );
        let s = of("padding: calc(50% * 2) calc(-10%)");
        assert_eq!(
            (&s.padding[TOP], &s.padding[RIGHT]),
            (
                &LengthPercentage::Percent(100.0),
                &LengthPercentage::Percent(0.0)
            )
        );
        assert_eq!(of("font-size: calc(50% + 2px)").font_size_px, 12.0);
        assert_eq!(of("font-size: calc(-1em)").font_size_px, 0.0);
        assert_eq!(of("z-index: calc(2.5)").z_index, Some(3));
        assert_eq!(of("z-index: calc(-2.5)").z_index, Some(-2));
        assert_eq!(of("z-index: calc(1px)").z_index, None, "invalid");
        assert_eq!(of("order: calc(infinity)").order, i32::MAX);
        assert_eq!(of("opacity: calc(100% / 4)").effects().opacity, 0.25);
        assert_eq!(of("opacity: calc(2)").effects().opacity, 1.0);
        // Percentages compared, as fractions of 1 (gap batch E).
        assert_eq!(of("opacity: min(50%, 0%)").effects().opacity, 0.0);
        assert_eq!(of("opacity: clamp(50%, 80%, 70%)").effects().opacity, 0.7);
        assert_eq!(of("opacity: calc(10% / 20%)").effects().opacity, 0.5);
        assert_eq!(of("border: calc(-1px) solid").border_width[TOP], 0.0);
        assert_eq!(of("letter-spacing: calc(1px - 3px)").letter_spacing, -2.0);
        assert_eq!(of("flex: calc(2) calc(-1) 5px").flex_shrink, 0.0);
        assert_eq!(
            of("width: calc(infinity * 1px)").width,
            Size::Px(MAX_LENGTH_PX)
        );
        assert!(matches!(
            of("vertical-align: calc(10% + 1px)").vertical_align,
            VerticalAlign::Calc(_)
        ));
        assert!(matches!(
            of("transform: translate(calc(50% - 1px))")
                .effects()
                .transform[..],
            [TransformFunction::Translate(
                LengthPercentage::Calc(_),
                _,
                _
            )]
        ));
        assert!(matches!(
            of("translate: calc(1px * 3) 2px").effects().translate,
            Some(Translate {
                x: LengthPercentage::Px(3.0),
                ..
            })
        ));
        assert_eq!(
            of("transform: rotate(calc(1turn / 4))").effects().transform,
            vec![TransformFunction::Rotate {
                x: 0.0,
                y: 0.0,
                z: 1.0,
                angle: 90.0
            }]
        );
        // A viewport unit inside one is a viewport unit used.
        let document = Document::from_node(&html::parse_document("<p>x</p>").expect("parses"));
        let sheets = [sheet("p { margin-left: calc(1px + 1vw) }")];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        assert!(styles.uses_viewport_units);
    }

    #[test]
    fn track_lists_take_sizes_line_names_and_repetitions() {
        use crate::computed::{
            RepeatCount, TrackBreadth as B, TrackRepeat, TrackSection as S, TrackSize as T,
        };
        let columns = |value: &str| grid_of(&format!("p {{ grid-template-columns: {value} }}"));
        let names =
            |names: &[&str]| -> Vec<Arc<str>> { names.iter().map(|&n| Arc::from(n)).collect() };
        let px = |v: f32| T::Breadth(B::Fixed(LengthPercentage::Px(v)));
        assert_eq!(columns("none").template_columns, None);
        let list = columns(
            "[a] 100px [b C] 1fr minmax(10%, max-content) fit-content(5em) auto min-content",
        )
        .template_columns
        .expect("valid");
        assert_eq!(
            list.names,
            vec![
                names(&["a"]),
                names(&["b", "C"]),
                vec![],
                vec![],
                vec![],
                vec![],
                vec![]
            ]
        );
        assert_eq!(
            list.sections,
            vec![
                S::Track(px(100.0)),
                S::Track(T::Breadth(B::Flex(1.0))),
                S::Track(T::MinMax(
                    B::Fixed(LengthPercentage::Percent(10.0)),
                    B::MaxContent
                )),
                S::Track(T::FitContent(LengthPercentage::Px(80.0))),
                S::Track(T::AUTO),
                S::Track(T::Breadth(B::MinContent)),
            ]
        );
        // A repetition keeps its own names and count; layout repeats it.
        let list = columns("repeat(2, [x] 10px [y]) [z] 2em")
            .template_columns
            .expect("valid");
        assert_eq!(list.names, vec![vec![], names(&["z"]), vec![]]);
        assert_eq!(
            list.sections,
            vec![
                S::Repeat(TrackRepeat {
                    count: RepeatCount::Count(2),
                    names: vec![names(&["x"]), names(&["y"])],
                    tracks: vec![px(10.0)],
                }),
                S::Track(px(32.0)),
            ]
        );
        // An automatic repetition among fixed sizes.
        let list = columns("10px repeat(auto-fill, minmax(100px, 1fr)) repeat(2, 5%)")
            .template_columns
            .expect("valid");
        assert_eq!(
            list.auto_repeat().map(|r| r.count),
            Some(RepeatCount::AutoFill)
        );
        let list = columns("repeat(auto-fit, minmax(auto, 10px))").template_columns;
        assert!(list.is_some(), "a fixed maximum makes a fixed size");
        // A count beyond a u32 is kept as the largest; layout limits it.
        let list = columns("repeat(99999999999, 1px)")
            .template_columns
            .expect("valid");
        assert!(matches!(
            &list.sections[0],
            S::Repeat(TrackRepeat {
                count: RepeatCount::Count(u32::MAX),
                ..
            })
        ));
        // Invalid: the declaration is dropped.
        for value in [
            "[a] [b] 10px",
            "[a]",
            "repeat(0, 10px)",
            "repeat(2.5, 10px)",
            "repeat(-1, 10px)",
            "repeat(2, repeat(2, 10px))",
            "repeat(2)",
            "repeat(2,)",
            "repeat(2, 10px",
            "repeat(auto-fill, 1fr)",
            "repeat(auto-fill, auto)",
            "repeat(auto-fit, minmax(min-content, 1fr))",
            "repeat(auto-fill, 10px) repeat(auto-fit, 10px)",
            "repeat(auto-fill, 10px) auto",
            "repeat(auto-fill, 10px) repeat(2, 1fr)",
            "minmax(1fr, 10px)",
            "minmax(10px)",
            "minmax(10px 20px)",
            "fit-content(auto)",
            "fit-content(1fr)",
            "-10px",
            "-1fr",
            "10px [a",
            "[span] 10px",
            "[auto] 10px",
            "[inherit] 10px",
            "[1] 10px",
            "subgrid",
            "calc(1fr)",
            "10px 'a'",
            "10 px",
            "none 10px",
        ] {
            assert_eq!(columns(value).template_columns, None, "{value}");
        }
    }

    #[test]
    fn template_areas_are_rectangles_of_named_cells() {
        use crate::computed::NamedArea;
        let areas =
            |value: &str| grid_of(&format!("p {{ grid-template-areas: {value} }}")).template_areas;
        let area = |name: &str, rows: (u32, u32), columns: (u32, u32)| NamedArea {
            name: Arc::from(name),
            rows,
            columns,
        };
        let a = areas("\"head head\" \"nav main\" \"foot .\"").expect("valid");
        assert_eq!((a.rows, a.columns), (3, 2));
        assert_eq!(
            a.areas,
            vec![
                area("head", (0, 1), (0, 2)),
                area("nav", (1, 2), (0, 1)),
                area("main", (1, 2), (1, 2)),
                area("foot", (2, 3), (0, 1)),
            ]
        );
        // A run of dots is one null cell, and white space separates;
        // a name may start with a digit, and is kept as written.
        let a = areas("'a  ... B' '1st ...B'").expect("valid");
        assert_eq!((a.rows, a.columns), (2, 3));
        assert_eq!(
            a.areas,
            vec![
                area("a", (0, 1), (0, 1)),
                area("B", (0, 2), (2, 3)),
                area("1st", (1, 2), (0, 1)),
            ]
        );
        assert_eq!(areas("none"), None);
        for value in [
            "'a b' 'a'",
            "'a b a'",
            "'a b' 'b a'",
            "'a .' '. a'",
            "'a #'",
            "''",
            "'   '",
            "'a' none",
            "a",
            "'a' 10px",
        ] {
            assert_eq!(areas(value), None, "{value}");
        }
    }

    #[test]
    fn grid_template_sets_the_rows_the_columns_and_the_areas() {
        use crate::computed::{TrackBreadth as B, TrackSection as S, TrackSize as T};
        let names =
            |names: &[&str]| -> Vec<Arc<str>> { names.iter().map(|&n| Arc::from(n)).collect() };
        let sizes = |list: &Option<crate::computed::TrackList>| -> Vec<S> {
            list.as_ref().expect("a list").sections.clone()
        };
        let fr = S::Track(T::Breadth(B::Flex(1.0)));
        let auto = S::Track(T::AUTO);
        let g = grid_of("p { grid-template: auto 1fr / auto 1fr auto }");
        assert_eq!(sizes(&g.template_rows), vec![auto.clone(), fr.clone()]);
        assert_eq!(
            sizes(&g.template_columns),
            vec![auto.clone(), fr.clone(), auto.clone()]
        );
        assert_eq!(g.template_areas, None);
        // The areas form (§7.4's example): the names after one row and
        // before the next are one line's.
        let g = grid_of(
            "p { grid-template: [header-top] 'a a a' [header-bottom]
                                 [main-top] 'b b b' 1fr [main-bottom]
                                 / auto 1fr auto }",
        );
        let rows = g.template_rows.as_ref().expect("rows");
        assert_eq!(
            rows.names,
            vec![
                names(&["header-top"]),
                names(&["header-bottom", "main-top"]),
                names(&["main-bottom"]),
            ]
        );
        assert_eq!(rows.sections, vec![auto.clone(), fr.clone()]);
        assert_eq!(sizes(&g.template_columns).len(), 3);
        let areas = g.template_areas.expect("areas");
        assert_eq!((areas.rows, areas.columns, areas.areas.len()), (2, 3, 2));
        let g = grid_of("p { grid-template: 'a' 'b' }");
        assert_eq!(sizes(&g.template_rows), vec![auto.clone(), auto.clone()]);
        assert_eq!(g.template_columns, None);
        // `none` resets all three.
        let s = style_of(
            "<p>x</p>",
            &[sheet(
                "p { grid-template-areas: 'a'; grid-template-columns: 1px; grid-template: none }",
            )],
            "p",
        );
        assert_eq!(s.grid, None);
        for value in [
            "'a' / repeat(2, 1px)",
            "1px 'a'",
            "auto 1fr",
            "'a' 'b' / 1px / 2px",
            "'a' [x] [y] [z] 'b'",
            "'a' repeat(2, 1px)",
            "'a b' 'c'",
            "/ 1px",
        ] {
            let g = grid_of(&format!("p {{ grid-template: {value} }}"));
            assert_eq!(g, crate::computed::GridStyle::default(), "{value}");
        }
    }

    #[test]
    fn the_grid_shorthand_sets_a_template_or_a_flow() {
        use crate::computed::{GridAutoFlow, TrackBreadth as B, TrackSection as S, TrackSize as T};
        let fr = T::Breadth(B::Flex(1.0));
        let g = grid_of(
            "p { grid-auto-flow: column; grid-auto-rows: 5px; grid: auto-flow 1fr / 100px }",
        );
        assert_eq!(g.template_rows, None);
        assert_eq!(
            g.template_columns.expect("columns").sections,
            vec![S::Track(T::Breadth(B::Fixed(LengthPercentage::Px(100.0))))]
        );
        assert_eq!(g.auto_flow, GridAutoFlow::default());
        assert_eq!(
            (g.auto_rows, g.auto_columns),
            (vec![fr.clone()], vec![T::AUTO])
        );
        let g = grid_of("p { grid: none / auto-flow dense 1fr 2fr }");
        assert_eq!((&g.template_rows, &g.template_columns), (&None, &None));
        assert_eq!(
            g.auto_flow,
            GridAutoFlow {
                column: true,
                dense: true
            }
        );
        assert_eq!(g.auto_columns, vec![fr.clone(), T::Breadth(B::Flex(2.0))]);
        assert_eq!(g.auto_rows, vec![T::AUTO]);
        let g = grid_of("p { grid: dense auto-flow / 10px }");
        assert_eq!(
            g.auto_flow,
            GridAutoFlow {
                column: false,
                dense: true
            }
        );
        // A template resets the implicit properties.
        let g = grid_of("p { grid-auto-flow: column dense; grid: 'a' 10px / 20px }");
        assert_eq!(g.auto_flow, GridAutoFlow::default());
        assert!(g.template_areas.is_some());
        for value in [
            "auto-flow / auto-flow",
            "1fr auto-flow / 10px",
            "auto-flow 1fr dense / 10px",
            "auto-flow",
            "auto-flow dense dense / 1px",
            "10px / 20px / 30px",
            "auto-flow / repeat(2, 1px) auto-flow",
        ] {
            let g = grid_of(&format!("p {{ grid: {value} }}"));
            assert_eq!(g, crate::computed::GridStyle::default(), "{value}");
        }
        for (value, expected) in [
            ("row", (false, false)),
            ("column", (true, false)),
            ("dense", (false, true)),
            ("dense column", (true, true)),
        ] {
            let g = grid_of(&format!("p {{ grid-auto-flow: {value} }}"));
            assert_eq!((g.auto_flow.column, g.auto_flow.dense), expected, "{value}");
        }
        for value in ["row column", "dense dense", "auto-flow", "1px"] {
            let g = grid_of(&format!(
                "p {{ grid-auto-flow: column; grid-auto-flow: {value} }}"
            ));
            assert!(g.auto_flow.column, "{value}");
        }
        let g = grid_of("p { grid-auto-rows: 10px minmax(0, 1fr) }");
        assert_eq!(g.auto_rows.len(), 2);
        for value in ["repeat(2, 1px)", "[a] 1px", "none"] {
            let g = grid_of(&format!("p {{ grid-auto-rows: {value} }}"));
            assert_eq!(g.auto_rows, vec![T::AUTO], "{value}");
        }
    }

    #[test]
    fn grid_lines_are_auto_names_numbers_and_spans() {
        use crate::computed::GridLine as L;
        let name = |n: &str| Some(Arc::<str>::from(n));
        let line = |value: &str| grid_of(&format!("p {{ grid-row-start: {value} }}")).row_start;
        for (value, expected) in [
            ("auto", L::Auto),
            ("foo", L::Ident(Arc::from("foo"))),
            ("Foo", L::Ident(Arc::from("Foo"))),
            ("3", L::Line(3, None)),
            ("-1", L::Line(-1, None)),
            ("2 foo", L::Line(2, name("foo"))),
            ("foo -2", L::Line(-2, name("foo"))),
            ("span 2", L::Span(2, None)),
            ("span foo", L::Span(1, name("foo"))),
            ("2 span", L::Span(2, None)),
            ("span foo 3", L::Span(3, name("foo"))),
            ("foo 3 span", L::Span(3, name("foo"))),
            ("99999999999", L::Line(i32::MAX, None)),
            // M21.2: math functions' integers, rounded.
            ("calc(1 + 1)", L::Line(2, None)),
            ("foo calc(-2.6)", L::Line(-3, name("foo"))),
            ("span calc(2.5)", L::Span(3, None)),
            ("min(4, 5) span", L::Span(4, None)),
        ] {
            assert_eq!(line(value), expected, "{value}");
        }
        for value in [
            "0",
            "0 foo",
            "span 0",
            "span -1",
            "span",
            "span span",
            "2 3",
            "foo bar",
            "2 span foo",
            "span auto",
            "auto 2",
            "1.5",
            "span 2 3",
            "inherit 2",
            "a b c d",
            "'a'",
            "calc(0.4)",
            "span calc(0)",
            "calc(1px)",
            "calc(1) calc(2)",
        ] {
            assert_eq!(line(value), L::Auto, "{value}");
        }
        let both = |css: &str| {
            let g = grid_of(css);
            (g.row_start, g.row_end, g.column_start, g.column_end)
        };
        let foo = || L::Ident(Arc::from("foo"));
        assert_eq!(
            both("p { grid-row: foo }"),
            (foo(), foo(), L::Auto, L::Auto)
        );
        assert_eq!(
            both("p { grid-row: 2 }"),
            (L::Line(2, None), L::Auto, L::Auto, L::Auto)
        );
        assert_eq!(
            both("p { grid-column: 1 / span 2 }"),
            (L::Auto, L::Auto, L::Line(1, None), L::Span(2, None))
        );
        assert_eq!(both("p { grid-area: foo }"), (foo(), foo(), foo(), foo()));
        assert_eq!(
            both("p { grid-area: 1 / 2 }"),
            (L::Line(1, None), L::Auto, L::Line(2, None), L::Auto)
        );
        let bar = || L::Ident(Arc::from("bar"));
        assert_eq!(
            both("p { grid-area: foo / bar }"),
            (foo(), foo(), bar(), bar())
        );
        assert_eq!(
            both("p { grid-area: 1 / 2 / 3 / 4 }"),
            (
                L::Line(1, None),
                L::Line(3, None),
                L::Line(2, None),
                L::Line(4, None)
            )
        );
        for css in [
            "p { grid-area: 1 / 2 / 3 / 4 / 5 }",
            "p { grid-row: 1 / }",
            "p { grid-row: / 2 }",
            "p { grid-column: 1 / 2 / 3 }",
            "p { grid-area: 0 }",
        ] {
            assert_eq!(both(css), (L::Auto, L::Auto, L::Auto, L::Auto), "{css}");
        }
    }

    #[test]
    fn justify_items_and_justify_self_take_the_inline_axis_values() {
        use AlignPosition::*;
        let pos = |p, o| SelfAlignment::Position(p, o);
        let self_value =
            |value: &str| grid_of(&format!("p {{ justify-self: {value} }}")).justify_self;
        for (value, expected) in [
            ("auto", SelfAlignment::Auto),
            ("normal", SelfAlignment::Normal),
            ("stretch", SelfAlignment::Stretch),
            ("baseline", SelfAlignment::Baseline(BaselinePosition::First)),
            (
                "last baseline",
                SelfAlignment::Baseline(BaselinePosition::Last),
            ),
            ("safe left", pos(Left, OverflowAlign::Safe)),
            ("right", pos(Right, OverflowAlign::Default)),
            ("unsafe center", pos(Center, OverflowAlign::Unsafe)),
            ("self-start", pos(SelfStart, OverflowAlign::Default)),
            ("flex-end", pos(FlexEnd, OverflowAlign::Default)),
        ] {
            assert_eq!(self_value(value), expected, "{value}");
        }
        for value in [
            "legacy",
            "left right",
            "safe",
            "safe baseline",
            "space-between",
        ] {
            let g = grid_of(&format!("p {{ justify-self: end; justify-self: {value} }}"));
            assert_eq!(g.justify_self, pos(End, OverflowAlign::Default), "{value}");
        }
        let items = |value: &str| grid_of(&format!("p {{ justify-items: {value} }}")).justify_items;
        let legacy = |p| JustifyItems {
            value: pos(p, OverflowAlign::Default),
            legacy: true,
        };
        let plain = |value| JustifyItems {
            value,
            legacy: false,
        };
        for (value, expected) in [
            ("normal", JustifyItems::NORMAL),
            ("legacy", JustifyItems::NORMAL),
            ("legacy left", legacy(Left)),
            ("center legacy", legacy(Center)),
            ("right", plain(pos(Right, OverflowAlign::Default))),
            ("safe end", plain(pos(End, OverflowAlign::Safe))),
            ("stretch", plain(SelfAlignment::Stretch)),
        ] {
            assert_eq!(items(value), expected, "{value}");
        }
        for value in [
            "auto",
            "legacy auto",
            "legacy start",
            "legacy legacy",
            "legacy safe left",
        ] {
            let g = grid_of(&format!(
                "p {{ justify-items: end; justify-items: {value} }}"
            ));
            assert_eq!(
                g.justify_items,
                plain(pos(End, OverflowAlign::Default)),
                "{value}"
            );
        }
    }

    #[test]
    fn a_legacy_justify_items_passes_to_the_children() {
        // css-align-3 §6.1: `legacy`, the initial value, is the parent's
        // value when that carries `legacy`; `normal` otherwise.
        use AlignPosition::*;
        let child = |css: &str| {
            style_of("<div><p>x</p></div>", &[sheet(css)], "p")
                .grid()
                .justify_items
        };
        let legacy = |p| JustifyItems {
            value: SelfAlignment::Position(p, OverflowAlign::Default),
            legacy: true,
        };
        assert_eq!(
            child("div { justify-items: legacy center }"),
            legacy(Center)
        );
        assert_eq!(child("div { justify-items: center }"), JustifyItems::NORMAL);
        assert_eq!(
            child("div { justify-items: legacy right } p { justify-items: legacy }"),
            legacy(Right)
        );
        assert_eq!(
            child("div { justify-items: legacy right } p { justify-items: initial }"),
            legacy(Right)
        );
        assert_eq!(
            child("div { justify-items: legacy right } p { justify-items: start }"),
            JustifyItems {
                value: SelfAlignment::Position(Start, OverflowAlign::Default),
                legacy: false
            }
        );
        assert_eq!(
            child("div { justify-items: center } p { justify-items: inherit }"),
            JustifyItems {
                value: SelfAlignment::Position(Center, OverflowAlign::Default),
                legacy: false
            }
        );
        assert_eq!(
            child("div { justify-items: legacy left } p { place-items: start legacy }"),
            legacy(Left)
        );
    }

    #[test]
    fn the_grid_properties_take_the_css_wide_keywords_and_keep_no_initial_copy() {
        let child = |css: &str| style_of("<div><p>x</p></div>", &[sheet(css)], "p");
        let parent = "div { grid-template-columns: 10px; grid-row: 2 / 3; grid-auto-flow: dense;
                       justify-self: end }";
        let s = child(&format!(
            "{parent} p {{ grid-template-columns: inherit; grid-row: inherit;
                          grid-auto-flow: inherit; justify-self: inherit }}"
        ));
        let d = style_of("<div><p>x</p></div>", &[sheet(parent)], "div");
        assert_eq!(s.grid(), d.grid());
        // Back to initial values: no copy is kept.
        for css in [
            "p { grid-template-columns: 10px; grid-template-columns: initial }",
            "p { grid-area: 1 / 2; grid-area: unset }",
            "p { grid: 'a' / 10px; grid: initial }",
            "p { grid-row: auto; grid-column-end: auto; justify-self: auto }",
            "p { place-self: center; place-self: unset }",
            "p { grid-auto-rows: 1px; grid-auto-rows: auto }",
        ] {
            assert_eq!(child(css).grid, None, "{css}");
        }
        // `inherit` on a shorthand takes every longhand of it.
        let s = child("div { grid: 'a' 10px / 20px } p { grid-template: inherit }");
        assert!(s.grid().template_areas.is_some() && s.grid().template_columns.is_some());
    }

    #[test]
    fn the_legacy_gap_names_are_the_gap_properties() {
        // css-align-3 §8.3 (M18).
        let p = |css: &str| style_of("<p>x</p>", &[sheet(css)], "p");
        let s = p("p { grid-gap: 1px 2px }");
        assert_eq!(
            (s.row_gap, s.column_gap),
            (
                Some(LengthPercentage::Px(1.0)),
                Some(LengthPercentage::Px(2.0))
            )
        );
        let s = p("p { grid-row-gap: 3px; grid-column-gap: 4% }");
        assert_eq!(
            (s.row_gap, s.column_gap),
            (
                Some(LengthPercentage::Px(3.0)),
                Some(LengthPercentage::Percent(4.0))
            )
        );
        // One property under two names: the later declaration wins.
        let s = p("p { grid-gap: 5px; row-gap: 6px }");
        assert_eq!(s.row_gap, Some(LengthPercentage::Px(6.0)));
    }

    #[test]
    fn the_effects_properties_cascade_behind_one_pointer() {
        // M19: the transform, opacity and will-change properties, none
        // inherited, land in `effects`; an element that sets none has
        // no pointer at all.
        use crate::computed::{
            BackfaceVisibility, TransformBox, TransformFunction, TransformStyle, WillChange,
        };
        let html = "<div><p>x</p></div>";
        let s = style_of(
            html,
            &[sheet(
                "div { transform: translate(10px, 20%) rotate(45deg); transform-origin: left top;
                       transform-box: content-box; translate: 1px 2px 3px; rotate: y 90deg;
                       scale: 2; transform-style: preserve-3d; perspective: 800px;
                       perspective-origin: right 10px bottom 20px; backface-visibility: hidden;
                       opacity: 150%; will-change: transform, opacity, contents }",
            )],
            "div",
        );
        let e = s.effects();
        assert_eq!(
            e.transform,
            vec![
                TransformFunction::Translate(
                    LengthPercentage::Px(10.0),
                    LengthPercentage::Percent(20.0),
                    0.0
                ),
                TransformFunction::Rotate {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                    angle: 45.0
                },
            ]
        );
        let zero = LengthPercentage::Percent(0.0);
        assert_eq!(
            (&e.transform_origin.x, &e.transform_origin.y),
            (&zero, &zero)
        );
        assert_eq!(e.transform_box, TransformBox::ContentBox);
        assert_eq!(
            e.translate.clone().map(|t| (t.x, t.y, t.z)),
            Some((LengthPercentage::Px(1.0), LengthPercentage::Px(2.0), 3.0))
        );
        assert_eq!(
            e.rotate.map(|r| (r.x, r.y, r.z, r.angle)),
            Some((0.0, 1.0, 0.0, 90.0))
        );
        assert_eq!(e.scale, Some([2.0, 2.0, 1.0]));
        assert_eq!(e.transform_style, TransformStyle::Preserve3d);
        assert_eq!(e.perspective, Some(800.0));
        assert_eq!(
            (
                e.perspective_origin.x.clone(),
                e.perspective_origin.y.clone()
            ),
            (
                LengthPercentage::sum(100.0, -10.0),
                LengthPercentage::sum(100.0, -20.0)
            )
        );
        assert_eq!(e.backface_visibility, BackfaceVisibility::Hidden);
        assert_eq!(e.opacity, 1.0, "clamped");
        assert_eq!(
            e.will_change,
            WillChange::TRANSFORM
                .with(WillChange::OPACITY)
                .with(WillChange::CONTENTS)
        );
        assert!(e.is_stacking_context() && e.contains_all_descendants());
        // Nothing is inherited.
        let child = style_of(
            html,
            &[sheet(
                "div { transform: scale(2); opacity: 0.5; will-change: transform }",
            )],
            "p",
        );
        assert_eq!(child.effects, None);
        assert!(!child.effects().is_stacking_context());
        // The `-webkit-` names are the properties.
        let s = style_of(
            html,
            &[sheet(
                "p { -webkit-transform: scale(2); -webkit-transform-origin: 0 0;
                     -webkit-transform-style: preserve-3d; -webkit-perspective: 1px;
                     -webkit-perspective-origin: 0 0; -webkit-backface-visibility: hidden }",
            )],
            "p",
        );
        let e = s.effects();
        assert_eq!(e.transform, vec![TransformFunction::Scale(2.0, 2.0, 1.0)]);
        // `0` is a length, where `left` was a percentage.
        assert_eq!(e.transform_origin.x, LengthPercentage::Px(0.0));
        assert_eq!(e.transform_style, TransformStyle::Preserve3d);
        assert_eq!(e.perspective, Some(1.0));
        assert_eq!(e.perspective_origin.y, LengthPercentage::Px(0.0));
        assert_eq!(e.backface_visibility, BackfaceVisibility::Hidden);
        // A later invalid declaration leaves the earlier value.
        let s = style_of(
            html,
            &[sheet(
                "p { opacity: 0.25; opacity: 1px; transform: scale(2); transform: bogus(1) }",
            )],
            "p",
        );
        assert_eq!(s.effects().opacity, 0.25);
        assert_eq!(s.effects().transform.len(), 1);
    }

    #[test]
    fn the_effects_properties_take_the_css_wide_keywords_and_keep_no_initial_copy() {
        let child = |css: &str| style_of("<div><p>x</p></div>", &[sheet(css)], "p");
        let parent = "div { transform: rotate(10deg); transform-origin: 1px 2px; opacity: 0.5;
                       will-change: scroll-position; perspective: 100px; rotate: 30deg }";
        let s = child(&format!(
            "{parent} p {{ transform: inherit; transform-origin: inherit; opacity: inherit;
                          will-change: inherit; perspective: inherit; rotate: inherit }}"
        ));
        let d = style_of("<div><p>x</p></div>", &[sheet(parent)], "div");
        assert_eq!(s.effects(), d.effects());
        // `unset` is `initial` for every one of them.
        let s = child(&format!(
            "{parent} p {{ transform: unset; opacity: unset; will-change: unset }}"
        ));
        assert_eq!(s.effects, None);
        // Back to initial values: no copy is kept.
        for css in [
            "p { transform: scale(2); transform: initial }",
            "p { opacity: 0.5; opacity: 1 }",
            "p { will-change: transform; will-change: auto }",
            "p { translate: 1px; translate: none; perspective: 1px; perspective: none }",
            "p { transform-origin: 50% 50%; transform-box: view-box; scale: none }",
            "p { opacity: 2 }",
        ] {
            assert_eq!(child(css).effects, None, "{css}");
        }
        let s = child("p { opacity: 0.5; transform: initial }");
        assert_eq!(s.effects().opacity, 0.5);
    }

    /// The color of the first `p` of `html` under `media`, `sources` its
    /// sheets in order.
    fn color_in(html: &str, sources: &[&str], media: Media) -> Color {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let sheets: Vec<css::Stylesheet> = sources.iter().map(|source| sheet(source)).collect();
        let context = CascadeContext {
            viewport_width: media.width,
            viewport_height: media.height,
            media,
            ..CascadeContext::default()
        };
        cascade_document(&document, &sheets, &context)
            .get(first(&document, "p"))
            .expect("styled")
            .color
    }

    fn print(width: f32, height: f32) -> Media {
        Media {
            media_type: crate::MediaType::Print,
            ..Media::screen(width, height)
        }
    }

    const RED: Color = Color::rgb8(255, 0, 0);
    const GREEN: Color = Color::rgb8(0, 128, 0);
    const BLUE: Color = Color::rgb8(0, 0, 255);

    #[test]
    fn media_blocks_apply_where_their_lists_hold() {
        let source = "p { color: red }
            @media (min-width: 600px) { p { color: blue } }
            @media print { p { color: green } }
            @media screen { @media (max-width: 300px) { p { color: green } } }
            @media print { @media (max-width: 300px) { p { color: black } } }";
        let at = |media| color_in("<p>x</p>", &[source], media);
        assert_eq!(at(Media::screen(800.0, 600.0)), BLUE);
        assert_eq!(at(Media::screen(500.0, 600.0)), RED);
        assert_eq!(at(print(800.0, 600.0)), GREEN);
        // Nested: every list around a rule must hold.
        assert_eq!(at(Media::screen(200.0, 600.0)), GREEN);
        assert_eq!(at(print(200.0, 600.0)), Color::rgb8(0, 0, 0));
        // A list that does not parse is `not all`; an empty one holds.
        let lists = "@media screen and { p { color: red } } @media { p { color: blue } }";
        assert_eq!(
            color_in("<p>x</p>", &[lists], Media::screen(1.0, 1.0)),
            BLUE
        );
        // A group rule nested in a style rule (CSS Nesting 1) applies its
        // declarations to the parent's elements where its list holds.
        let nested = "p { color: blue; @media all { color: red } }";
        assert_eq!(color_in("<p>x</p>", &[nested], Media::default()), RED);
        let nested = "p { color: blue; @media print { color: red } }";
        assert_eq!(color_in("<p>x</p>", &[nested], Media::default()), BLUE);
    }

    #[test]
    fn supports_blocks_are_decided_by_what_the_cascade_takes() {
        let at = |source: &str| color_in("<p>x</p>", &[source], Media::default());
        assert_eq!(at("@supports (display: grid) { p { color: blue } }"), BLUE);
        assert_eq!(
            at("p { color: blue } @supports (display: nonsense) { p { color: red } }"),
            BLUE
        );
        assert_eq!(
            at("@supports not (display: nonsense) { p { color: blue } }"),
            BLUE
        );
        assert_eq!(
            at("@supports selector(p > a) and (color: red) { p { color: blue } }"),
            BLUE
        );
        assert_eq!(
            at("p { color: blue } @supports selector(:is(:foo)) { p { color: red } }"),
            BLUE
        );
        // A condition that does not parse drops the rule and its block.
        assert_eq!(
            at("p { color: blue } @supports (a: b) or (c: d) and (e: f) { p { color: red } }"),
            BLUE
        );
        assert_eq!(
            at("p { color: blue } @supports display: grid { p { color: red } }"),
            BLUE
        );
        // Inside `@media`, and `@media` inside it.
        assert_eq!(
            at("@media all { @supports (color: red) { @media all { p { color: blue } } } }"),
            BLUE
        );
        // `@container` and `@scope` are passed over with their rules.
        assert_eq!(
            at(
                "p { color: blue } @container (width > 0) { p { color: red } }\
                @scope (p) { p { color: red } }"
            ),
            BLUE
        );
    }

    #[test]
    fn nested_rules_and_scopes_match_as_desugared() {
        let html = "<div class=a><p id=x class=c>x</p></div><p class=c>y</p>";
        let at = |source: &str| color_in(html, &[source], Media::default());
        // D71: `&` the parent's selectors, implicit before a relative or
        // `&`-less selector, with the parent's specificity.
        assert_eq!(at(".a { .c { color: blue } }"), BLUE);
        assert_eq!(at(".a { > .c { color: blue } }"), BLUE);
        assert_eq!(at(".c { .a > & { color: blue } }"), BLUE);
        assert_eq!(at(".z { .c { color: blue } }"), Color::BLACK);
        assert_eq!(
            at("#x { color: blue } .a { #x.c { color: red } }"),
            RED,
            "`:is(.a) #x.c` outweighs `#x`"
        );
        // Declarations after a nested rule come after it.
        assert_eq!(at(".c { & { color: red } color: blue }"), BLUE);
        // A group rule nested applies its rules where its list holds.
        assert_eq!(at(".c { @media print { color: red } }"), Color::BLACK);
        // `@scope`: within the root, not past a limit, the nearer root
        // winning at equal specificity whatever the order.
        assert_eq!(at("@scope (.a) { .c { color: blue } }"), BLUE);
        assert_eq!(
            at("@scope (.a) to (.c) { .c { color: blue } }"),
            Color::BLACK
        );
        assert_eq!(
            at("@scope (div) { .c { color: blue } } @scope (body) { .c { color: red } }"),
            BLUE
        );
        assert_eq!(
            at("@scope (body) { .c { color: red } } @scope (div) { .c { color: blue } }"),
            BLUE
        );
        assert_eq!(
            at("@scope (div) { .c { color: red } } body .c { color: blue }"),
            BLUE,
            "specificity before proximity"
        );
        // Selecting the root takes `:scope`.
        assert_eq!(at("@scope (.c) { .c { color: red } }"), Color::BLACK);
        assert_eq!(at("@scope (.c) { :scope { color: blue } }"), BLUE);
    }

    /// The value of the custom property `name` on the first element
    /// `selector` matches in `html` styled by `source`, with `registered`
    /// a script's registrations.
    fn custom_value(
        html: &str,
        source: &str,
        registered: Vec<PropertyRegistration>,
        selector: &str,
        name: &str,
    ) -> Option<String> {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let sheets = Stylesheets::new(vec![sheet(source)]).with_registered(registered);
        let styles = sheets.cascade(&document, &CascadeContext::default());
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        let element = document
            .subtree(document.root())
            .find(|&id| matches_selector(&document, id, &list.0[0]))
            .expect("matched");
        crate::serialize::computed_value(styles.get(element).expect("styled"), name)
    }

    #[test]
    fn registered_properties_compute_by_their_syntax() {
        let length = |inherits| {
            crate::property::register_property("--l", "<length>", inherits, Some("1in"))
                .expect("valid")
        };
        let html = "<div id=a style='font-size: 10px'><p id=b></p></div>";
        // D73: its initial value, computed, where nothing declares it.
        assert_eq!(
            custom_value(html, "", vec![length(false)], "p", "--l").as_deref(),
            Some("96px")
        );
        // Computed by its syntax against the element's font.
        assert_eq!(
            custom_value(html, "#a { --l: 3em }", vec![length(true)], "#a", "--l").as_deref(),
            Some("30px")
        );
        // Inherited as computed, or not inherited at all.
        assert_eq!(
            custom_value(html, "#a { --l: 3em }", vec![length(true)], "p", "--l").as_deref(),
            Some("30px")
        );
        assert_eq!(
            custom_value(html, "#a { --l: 3em }", vec![length(false)], "p", "--l").as_deref(),
            Some("96px")
        );
        // A value not of its syntax is invalid at computed-value time.
        assert_eq!(
            custom_value(html, "#a { --l: red }", vec![length(false)], "#a", "--l").as_deref(),
            Some("96px")
        );
        // An `@property` rule registers it too, a script's registration
        // winning over it.
        let rule = "@property --l { syntax: '<color>'; inherits: false; initial-value: red }";
        assert_eq!(
            custom_value(html, rule, Vec::new(), "p", "--l").as_deref(),
            Some("rgb(255, 0, 0)")
        );
        assert_eq!(
            custom_value(html, rule, vec![length(false)], "p", "--l").as_deref(),
            Some("96px")
        );
        // A colour's `currentcolor` is the element's colour.
        let color = crate::property::register_property("--c", "<color>", false, Some("red"))
            .expect("valid");
        assert_eq!(
            custom_value(
                html,
                "#a { color: blue; --c: currentcolor }",
                vec![color],
                "#a",
                "--c"
            )
            .as_deref(),
            Some("rgb(0, 0, 255)")
        );
        // `var()` substitutes the computed value.
        assert_eq!(
            custom_value(
                html,
                "#a { --l: 2em; --u: var(--l) }",
                vec![length(false)],
                "#a",
                "--u"
            )
            .as_deref(),
            Some("20px")
        );
    }

    #[test]
    fn revert_rolls_back_to_the_user_agents_declarations() {
        // D73: an author's `revert` takes the user agent's value, the
        // author's other declarations however strong passed over.
        let style = |source: &str, selector: &str| {
            style_of(
                "<div id=d class=c>d</div><h1 id=h>h</h1><p id=p>p</p>",
                &[sheet(source)],
                selector,
            )
        };
        let div = style("#d.c { display: revert } div { display: inline }", "div");
        assert_eq!(div.display, Display::Block);
        let div = style(
            "div { display: inline !important } #d { display: revert !important }",
            "div",
        );
        assert_eq!(div.display, Display::Block, "the important ones too");
        let div = style("#d.c { display: inline } div { display: revert }", "div");
        assert_eq!(
            div.display,
            Display::Inline,
            "a weaker keyword is no winner"
        );
        // Every longhand of a shorthand, and of `all`.
        let h1 = style("h1 { margin: 0; margin: revert }", "h1");
        assert_eq!(h1.margin, style("", "h1").margin);
        assert_ne!(
            style("", "h1").margin,
            style("h1 { margin: 0 }", "h1").margin
        );
        let h1 = style("h1 { margin: 0; display: inline; all: revert }", "h1");
        assert_eq!(
            (h1.margin.clone(), h1.display),
            (style("", "h1").margin, Display::Block)
        );
        // The font size, with its own pass, and `color`, with its own.
        let h1 = style(
            "h1 { font-size: 10px; color: red } h1 { font-size: revert; color: revert }",
            "h1",
        );
        assert_eq!(h1.font_size_px, 32.0);
        assert_eq!(h1.color, Color::BLACK);
        // In the user agent's origin, nothing is below: `unset`.
        let p = style(
            "p { color: red; text-align: right } p { text-align: revert }",
            "p",
        );
        assert_eq!(p.text_align, ComputedStyle::default().text_align);
        // A custom property rolls back too, to none declared.
        let p = style("p { --x: 1 } p { --x: revert }", "p");
        assert!(p.custom.as_ref().is_none_or(|m| m.get("--x").is_none()));
    }

    #[test]
    fn revert_layer_rolls_back_to_the_layer_below() {
        let at = |source: &str| color_in("<p id=x class=c>x</p>", &[source], Media::default());
        // D73: to the layer before, the unlayered declarations being the
        // last layer.
        assert_eq!(
            at(
                "@layer a, b; @layer a { p { color: blue } } @layer b { #x { color: revert-layer } }"
            ),
            BLUE
        );
        assert_eq!(
            at("@layer a { p { color: blue } } #x.c { color: red } p { color: revert-layer }"),
            RED,
            "the unlayered declarations' own are one layer: the stronger one wins"
        );
        assert_eq!(
            at("@layer a { p { color: blue } } #x.c { color: revert-layer } p { color: red }"),
            BLUE,
            "the keyword the strongest: it rolls back past the weaker ones of its layer"
        );
        assert_eq!(
            at("@layer a { p { color: blue } } #x.c { color: revert-layer }"),
            BLUE
        );
        // `revert-rule`: to the rules below its own in the cascade's order.
        assert_eq!(
            at("p { color: blue } p { color: red; color: revert-rule }"),
            BLUE
        );
        assert_eq!(
            at("#x { color: red; color: revert-rule } .c { color: blue } p { color: red }"),
            BLUE,
            "the rule below by specificity, not by the order written"
        );
        // Past the first layer, to the user agent's.
        assert_eq!(
            at("@layer a { p { color: red; color: revert-layer } }"),
            Color::BLACK
        );
        // Among important declarations, the layer below is the next in
        // their order, the normal declarations below every one.
        assert_eq!(
            at(
                "@layer a, b; @layer a { p { color: revert-layer !important } } \
                @layer b { p { color: green !important } } p { color: blue }"
            ),
            GREEN
        );
        assert_eq!(
            at("@layer a { p { color: revert-layer !important } } p { color: blue }"),
            BLUE
        );
        // A custom property, and a value substituted to one.
        let p = style_of(
            "<p>x</p>",
            &[sheet(
                "@layer a { p { --x: 1 } } p { --x: 2; --x: revert-layer }",
            )],
            "p",
        );
        assert_eq!(
            p.custom
                .as_ref()
                .and_then(|m| m.get("--x"))
                .map(|v| v.to_vec()),
            Some(css::tokenize("1"))
        );
        assert_eq!(
            at("@layer a { p { color: blue } } p { --k: revert-layer; color: var(--k) }"),
            Color::BLACK,
            "a custom property rolled back to none: `var()` of it invalid, `color` unset"
        );
        assert_eq!(
            at("@layer a { p { color: blue } } p { color: var(--none, revert-layer) }"),
            BLUE,
            "a fallback substituted"
        );
    }

    #[test]
    fn layers_order_normal_declarations_and_reverse_important_ones() {
        let at = |source: &str| color_in("<p id=x class=c>x</p>", &[source], Media::default());
        // Later layers win, whatever the specificity or the order.
        assert_eq!(
            at("@layer a, b; @layer b { p { color: blue } } @layer a { #x { color: red } }"),
            BLUE
        );
        // Rules in no layer win over every layer.
        assert_eq!(
            at("p { color: blue } @layer a { #x.c { color: red } }"),
            BLUE
        );
        // Among important declarations the order is reversed: the first
        // layer wins, and rules in no layer lose.
        assert_eq!(
            at("@layer a { p { color: blue !important } }\
                @layer b { #x { color: red !important } }\
                #x.c { color: red !important }"),
            BLUE
        );
        // A layer's own rules come after the layers inside it.
        assert_eq!(
            at("@layer a { p { color: blue } @layer x { #x { color: red } } }"),
            BLUE
        );
        assert_eq!(
            at(
                "@layer a { p { color: red !important } @layer x { p { color: blue !important } } }"
            ),
            BLUE
        );
        // A dotted name is a layer inside a layer; the order is that of
        // the first naming.
        assert_eq!(
            at("@layer a.x { #x { color: red } } @layer a { p { color: blue } }"),
            BLUE
        );
        assert_eq!(
            at(
                "@layer a.y, a.x; @layer a.x { p { color: blue } } @layer a.y { #x { color: red } }"
            ),
            BLUE
        );
        assert_eq!(
            at("@layer a { @layer x { p { color: red } } } @layer a.x { p { color: blue } }"),
            BLUE,
            "`a.x` names the layer `x` inside `a`"
        );
        // Anonymous layers are each their own, in order.
        assert_eq!(
            at("@layer { #x { color: red } } @layer { p { color: blue } }"),
            BLUE
        );
        // Names are case-sensitive.
        assert_eq!(
            at("@layer A, a; @layer a { p { color: blue } } @layer A { #x { color: red } }"),
            BLUE
        );
        // The inline style is above every layer, normal and important.
        let inline = color_in(
            "<p style='color: blue'>x</p>",
            &["@layer a { p { color: red } }"],
            Media::default(),
        );
        assert_eq!(inline, BLUE);
        let inline = color_in(
            "<p style='color: blue !important'>x</p>",
            &["@layer a { p { color: red !important } }"],
            Media::default(),
        );
        assert_eq!(inline, BLUE);
        // A layered author rule still beats the UA sheet.
        let link = color_in(
            "<p><a href=x>x</a></p>",
            &["@layer a { p { color: blue } }"],
            Media::default(),
        );
        assert_eq!(link, BLUE);
    }

    #[test]
    fn a_statement_in_one_sheet_orders_the_layers_of_the_next() {
        let sheets = [
            "@layer base, theme;",
            "@layer theme { p { color: blue } } @layer base { #x { color: red } }",
        ];
        assert_eq!(color_in("<p id=x>x</p>", &sheets, Media::default()), BLUE);
        let reversed = ["@layer theme, base;", sheets[1]];
        assert_eq!(color_in("<p id=x>x</p>", &reversed, Media::default()), RED);
    }

    #[test]
    fn a_layer_named_only_where_the_media_does_not_hold_takes_no_place() {
        // Cascade 5 §6.4.3: the statement inside `@media print` orders
        // `b` before `a` on paper alone.
        let source = "@media print { @layer b; }
            @layer a { p { color: red } } @layer b { p { color: blue } }";
        assert_eq!(color_in("<p>x</p>", &[source], Media::default()), BLUE);
        assert_eq!(color_in("<p>x</p>", &[source], print(800.0, 600.0)), RED);
        // The web-platform-tests' (layer-media-query.html, C3).
        let source = "@media (max-width: 300px) { @layer B, A; }
            @media (min-width: 500px) { @layer A, B; }
            @layer A { p { color: red } } @layer B { p { color: green } }";
        assert_eq!(
            color_in("<p>x</p>", &[source], Media::screen(300.0, 300.0)),
            RED
        );
        assert_eq!(
            color_in("<p>x</p>", &[source], Media::screen(500.0, 300.0)),
            GREEN
        );
        // Inside a false `@supports`, never.
        let source = "@supports (nonsense: 1) { @layer b; }
            @layer a { p { color: red } } @layer b { p { color: blue } }";
        assert_eq!(color_in("<p>x</p>", &[source], Media::default()), BLUE);
    }

    #[test]
    fn an_invalid_layer_rule_is_dropped_with_its_block() {
        let at = |source: &str| color_in("<p>x</p>", &[source], Media::default());
        for bad in [
            "@layer initial { p { color: red } }",
            "@layer a.revert-layer { p { color: red } }",
            "@layer a b { p { color: red } }",
            "@layer a, b { p { color: red } }",
            "@layer a. { p { color: red } }",
            "@layer .a { p { color: red } }",
            "@layer a .b { p { color: red } }",
            "@layer 1 { p { color: red } }",
            // The web-platform-tests' (layer-rule-invalid-name.html).
            "@layer 123 { p { color: red } }",
            "@layer foo.123 { p { color: red } }",
            "@layer - { p { color: red } }",
        ] {
            assert_eq!(at(&format!("p {{ color: blue }} {bad}")), BLUE, "{bad}");
        }
        // A statement with one bad name names none of them. The draft
        // reserves the CSS-wide keywords, which the reference browser
        // takes as names.
        assert_eq!(
            at("@layer b, unset; @layer a { #x, p { color: blue } } @layer b { p { color: red } }"),
            RED,
            "`b` is not ordered first"
        );
    }

    #[test]
    fn styles_keep_the_answers_that_chose_their_rules() {
        let document = Document::from_node(&html::parse_document("<p>x</p>").expect("never fails"));
        let sheets = Stylesheets::new(vec![sheet(
            "@media (min-width: 600px) { p { color: blue } } @media print { p { color: red } }",
        )]);
        let wide = Media::screen(800.0, 600.0);
        let context = CascadeContext {
            media: wide,
            ..CascadeContext::default()
        };
        let mut styles = sheets.cascade(&document, &context);
        assert!(!sheets.media_changed(&styles, &wide));
        assert!(!sheets.media_changed(&styles, &Media::screen(1000.0, 10.0)));
        assert!(sheets.media_changed(&styles, &Media::screen(500.0, 600.0)));
        assert!(sheets.media_changed(&styles, &print(800.0, 600.0)));
        // A restyle under other answers is a full cascade.
        let narrow = CascadeContext {
            media: Media::screen(500.0, 600.0),
            ..CascadeContext::default()
        };
        let restyle = sheets.restyle(&document, &narrow, &mut styles, &[]);
        assert!(restyle.full);
        assert!(!sheets.media_changed(&styles, &Media::screen(500.0, 600.0)));
        let p = first(&document, "p");
        assert_eq!(styles.get(p).expect("styled").color, Color::rgb8(0, 0, 0));
        let restyle = sheets.restyle(&document, &narrow, &mut styles, &[]);
        assert!(!restyle.full, "the same answers: nothing to do");
    }
    #[test]
    fn the_break_properties_and_their_legacy_names_cascade() {
        use crate::computed::{BreakBetween, BreakInside, PrintColorAdjust};
        let sheets = [sheet(
            "div { break-before: page; break-after: avoid-page; break-inside: avoid;
                   orphans: 3; widows: 4; -webkit-print-color-adjust: exact }
             p { page-break-before: always; page-break-after: left; page-break-inside: avoid }
             span { orphans: inherit; widows: initial; print-color-adjust: unset;
                    break-before: inherit }
             b { page-break-before: recto; break-after: always; orphans: 0 }
             i { break-before: page; break-before: initial }",
        )];
        let html = "<div><p>x</p><span>s</span><b>b</b><i>i</i></div>";
        let div = style_of(html, &sheets, "div");
        let f = div.fragmentation();
        assert_eq!(f.break_before, BreakBetween::Page);
        assert_eq!(f.break_after, BreakBetween::AvoidPage);
        assert_eq!(f.break_inside, BreakInside::Avoid);
        assert_eq!((f.orphans, f.widows), (3, 4));
        assert_eq!(f.print_color_adjust, PrintColorAdjust::Exact);
        // The legacy names, `always` being `page`; the inherited ones
        // from the div.
        let p = style_of(html, &sheets, "p");
        let f = p.fragmentation();
        assert_eq!(f.break_before, BreakBetween::Page);
        assert_eq!(f.break_after, BreakBetween::Left);
        assert_eq!(f.break_inside, BreakInside::Avoid);
        assert_eq!((f.orphans, f.widows), (3, 4));
        assert_eq!(f.print_color_adjust, PrintColorAdjust::Exact);
        // `inherit` takes even a property that is not inherited; `unset`
        // of an inherited one is `inherit`.
        let span = style_of(html, &sheets, "span");
        let f = span.fragmentation();
        assert_eq!(f.break_before, BreakBetween::Page);
        assert_eq!((f.orphans, f.widows), (3, 2));
        assert_eq!(f.print_color_adjust, PrintColorAdjust::Exact);
        // Values the properties do not take are dropped.
        let b = style_of(html, &sheets, "b");
        assert_eq!(b.fragmentation().break_before, BreakBetween::Auto);
        assert_eq!(b.fragmentation().break_after, BreakBetween::Auto);
        assert_eq!(b.fragmentation().orphans, 3);
        // Set back to the initial values, the style keeps no copy of
        // them but what it inherits.
        let html = "<i>i</i>";
        assert_eq!(style_of(html, &sheets, "i").fragmentation, None);
        for (name, value) in [
            ("break-before", "region"),
            ("break-inside", "avoid-column"),
            ("page-break-after", "always"),
            ("orphans", "1"),
            ("widows", "10"),
            ("print-color-adjust", "economy"),
            ("-webkit-print-color-adjust", "exact"),
        ] {
            assert!(supports(name, &css::tokenize(value)), "{name}: {value}");
        }
        assert!(!supports("break-before", &css::tokenize("always")));
        assert!(!supports("orphans", &css::tokenize("0")));
    }
    #[test]
    fn page_rules_cascade_for_each_page() {
        use crate::page::{Orientation, PagePosition, PageSize};
        let print = |width, height| Media {
            media_type: crate::MediaType::Print,
            ..Media::screen(width, height)
        };
        let px = LengthPercentageAuto::Px;
        let first = PagePosition {
            first: true,
            ..PagePosition::default()
        };
        let left = PagePosition {
            left: true,
            ..PagePosition::default()
        };
        let right = PagePosition::default();
        let style = |source: &str, media: Media, at: PagePosition| {
            Stylesheets::new(vec![sheet(source)]).page_style(&media, 16.0, at)
        };
        let paper = print(816.0, 1056.0);

        // Nothing said: the print's own.
        let none = style("p { color: red }", paper, first);
        assert_eq!(none.size, PageSize::Auto);
        assert_eq!(none.margin, <[Option<LengthPercentageAuto>; 4]>::default());

        let source = "@page { size: A4; margin: 1in }
            @page :left { margin-left: 2in; margin-right: 10% }
            @page :first { margin-top: 3em; size: letter landscape }
            @page :right:first { margin-bottom: auto }
            @page cover { margin: 0 }";
        let s = style(source, paper, first);
        assert_eq!(
            s.size,
            PageSize::Exact {
                width: 1056.0,
                height: 816.0
            }
        );
        assert_eq!(
            s.margin,
            [
                Some(px(48.0)),
                Some(px(96.0)),
                Some(LengthPercentageAuto::Auto),
                Some(px(96.0))
            ]
        );
        let s = style(source, paper, left);
        let (width, height) = s.size.resolve((0.0, 0.0));
        assert!(
            (width - 793.7).abs() < 0.1 && (height - 1122.5).abs() < 0.1,
            "A4"
        );
        assert_eq!(
            s.margin,
            [
                Some(px(96.0)),
                Some(LengthPercentageAuto::Percent(10.0)),
                Some(px(96.0)),
                Some(px(192.0))
            ]
        );
        assert_eq!(
            style(source, paper, right).margin,
            std::array::from_fn(|_| Some(px(96.0)))
        );

        // Specificity over order; importance over specificity; layers.
        let s = style(
            "@page :first { margin-top: 1px } @page { margin-top: 2px }",
            paper,
            first,
        );
        assert_eq!(s.margin[TOP], Some(px(1.0)));
        let s = style(
            "@page { margin-top: 2px !important } @page :first { margin-top: 1px }",
            paper,
            first,
        );
        assert_eq!(s.margin[TOP], Some(px(2.0)));
        let s = style(
            "@layer a, b; @layer b { @page { margin-top: 2px } } @layer a { @page :first { margin-top: 1px } }",
            paper,
            first,
        );
        assert_eq!(
            s.margin[TOP],
            Some(px(2.0)),
            "a later layer wins whatever the specificity"
        );
        let s = style(
            "@layer a { @page { margin-top: 1px } } @page { margin-top: 2px }",
            paper,
            first,
        );
        assert_eq!(s.margin[TOP], Some(px(2.0)));

        // Inside `@media`, by the medium.
        let source = "@page { size: portrait } @media print and (min-width: 800px) { @page { size: landscape } }";
        assert_eq!(
            style(source, paper, right).size,
            PageSize::Oriented(Orientation::Landscape)
        );
        assert_eq!(
            style(source, print(500.0, 800.0), right).size,
            PageSize::Oriented(Orientation::Portrait)
        );
        assert_eq!(
            style(source, Media::screen(1000.0, 800.0), right).size,
            PageSize::Oriented(Orientation::Portrait)
        );

        // Values the descriptors do not take are dropped; a CSS-wide
        // keyword leaves them to the print.
        for bad in [
            "10%",
            "a0",
            "a4 a4",
            "landscape portrait",
            "auto landscape",
            "-1cm",
            "1px 2px 3px",
            "10cm portrait",
        ] {
            let s = style(
                &format!("@page {{ size: 5in }} @page {{ size: {bad} }}"),
                paper,
                right,
            );
            assert_eq!(
                s.size,
                PageSize::Exact {
                    width: 480.0,
                    height: 480.0
                },
                "{bad}"
            );
        }
        let s = style(
            "@page { size: 4in 6in; margin: 1in } @page { size: initial; margin-left: inherit }",
            paper,
            right,
        );
        assert_eq!(s.size, PageSize::Auto);
        assert_eq!(
            s.margin,
            [Some(px(96.0)), Some(px(96.0)), Some(px(96.0)), None]
        );
        assert_eq!(
            style("@page { size: 2em 1rem }", paper, right).size,
            PageSize::Exact {
                width: 32.0,
                height: 16.0
            }
        );
        assert_eq!(
            style("@page { size: landscape JIS-B5 }", paper, right)
                .size
                .resolve((0.0, 0.0))
                .1
                .round(),
            688.0
        );
        // A rule whose selectors do not parse is dropped; the page-margin
        // boxes inside a rule are passed over.
        let s = style(
            "@page :nth(1) { margin: 0 } @page { margin: 1px; @top-left { content: 'x'; margin: 9px } }",
            paper,
            first,
        );
        assert_eq!(s.margin, std::array::from_fn(|_| Some(px(1.0))));
    }

    /// The style of the first `p` in `html` with `css` as its one sheet.
    fn p_with(html: &str, css: &str) -> ComputedStyle {
        style_of(html, &[sheet(css)], "p")
    }

    #[test]
    fn the_scroll_snap_properties_parse_and_serialize() {
        let value = |css: &str, name: &str| {
            let document = Document::from_node(&html::parse_document("<p>x</p>").unwrap());
            let styles = cascade_document(&document, &[sheet(css)], &CascadeContext::default());
            crate::serialize::computed_value(styles.get(first(&document, "p")).unwrap(), name)
                .unwrap()
        };
        for (written, name, computed) in [
            ("", "scroll-snap-type", "none"),
            (
                "scroll-snap-type: Y Mandatory",
                "scroll-snap-type",
                "y mandatory",
            ),
            (
                "scroll-snap-type: both proximity",
                "scroll-snap-type",
                "both",
            ),
            ("scroll-snap-type: inline", "scroll-snap-type", "inline"),
            ("", "scroll-snap-align", "none"),
            ("scroll-snap-align: start", "scroll-snap-align", "start"),
            ("scroll-snap-align: end end", "scroll-snap-align", "end"),
            (
                "scroll-snap-align: center start",
                "scroll-snap-align",
                "center start",
            ),
            ("", "scroll-snap-stop", "normal"),
            ("scroll-snap-stop: ALWAYS", "scroll-snap-stop", "always"),
            // Scroll Snap 2's, as the reference browser takes them.
            ("scroll-snap-type: pair", "scroll-snap-type", "pair"),
            (
                "scroll-snap-type: pair mandatory",
                "scroll-snap-type",
                "pair mandatory",
            ),
            ("", "scroll-initial-target", "none"),
            (
                "scroll-initial-target: Nearest",
                "scroll-initial-target",
                "nearest",
            ),
        ] {
            assert_eq!(
                value(&format!("p {{ {written} }}"), name),
                computed,
                "{written}"
            );
        }
        for (name, invalid) in [
            ("scroll-snap-type", "mandatory"),
            ("scroll-snap-type", "none mandatory"),
            ("scroll-snap-type", "x y"),
            ("scroll-snap-align", "start start start"),
            ("scroll-snap-align", "left"),
            ("scroll-snap-stop", "none"),
            ("scroll-snap-stop", "before"),
            ("scroll-snap-type", "both pair"),
            ("scroll-snap-type", "pair x"),
            ("scroll-initial-target", "auto"),
            ("scroll-initial-target", "nearest none"),
        ] {
            assert!(
                !supports(name, &css::tokenize(invalid)),
                "{name}: {invalid}"
            );
        }
        // Not inherited.
        let document = Document::from_node(&html::parse_document("<div><p>x</p></div>").unwrap());
        let styles = cascade_document(
            &document,
            &[sheet(
                "div { scroll-snap-type: y; scroll-snap-align: start;
                       scroll-initial-target: nearest }",
            )],
            &CascadeContext::default(),
        );
        let p = styles.get(first(&document, "p")).unwrap();
        assert_eq!(p.effects().scroll_snap_type, ScrollSnapType::NONE);
        assert_eq!(p.effects().scroll_snap_align, [SnapAlign::None; 2]);
        assert!(!p.effects().scroll_initial_target);
    }

    #[test]
    fn scroll_margin_and_scroll_padding_are_four_sides_each() {
        let p = p_with(
            "<p>x</p>",
            "p { font-size: 10px; scroll-margin: 1px 2em; scroll-padding: 5% auto 3px;              scroll-margin-left: 4px }",
        );
        assert_eq!(p.effects().scroll_margin, [1.0, 20.0, 1.0, 4.0]);
        assert_eq!(
            p.effects().scroll_padding,
            [
                LengthPercentageAuto::Percent(5.0),
                LengthPercentageAuto::Auto,
                LengthPercentageAuto::Px(3.0),
                LengthPercentageAuto::Auto
            ]
        );
        assert!(!supports("scroll-padding", &css::tokenize("-1px")));
        assert!(!supports("scroll-margin", &css::tokenize("10%")));
        let value = |name: &str| crate::serialize::computed_value(&p, name);
        assert_eq!(value("scroll-margin-right").as_deref(), Some("20px"));
        assert_eq!(value("scroll-padding-top").as_deref(), Some("5%"));
    }

    #[test]
    fn all_sets_every_longhand_but_direction_and_unicode_bidi() {
        let p = p_with(
            "<div style='color: blue; width: 10px; font-size: 20px'><p dir=rtl>x</p></div>",
            "p { color: red; width: 5px; font-size: 8px; all: inherit; height: 3px }",
        );
        assert_eq!(p.color, Color::rgb8(0, 0, 255));
        assert_eq!(p.width, Size::Px(10.0));
        assert_eq!(p.font_size_px, 20.0);
        assert_eq!(p.height, Size::Px(3.0));
        assert_eq!(p.direction, Direction::Rtl);
        let p = p_with(
            "<p>x</p>",
            "p { all: initial; color: green; display: block }",
        );
        assert_eq!(p.color, Color::rgb8(0, 128, 0));
        assert_eq!(p.display, Display::Block);
        let p = p_with("<p>x</p>", "p { margin: 3px; all: initial }");
        assert_eq!(p.margin[LEFT], LengthPercentageAuto::Px(0.0));
        assert!(supports("all", &css::tokenize("unset")));
        assert!(!supports("all", &css::tokenize("red")));
    }

    #[test]
    fn custom_properties_are_substituted_into_properties() {
        // M21.1: declared, inherited, substituted.
        let p = p_with(
            "<div><p>x</p></div>",
            ":root { --main: #008000; --gap: 4px } div { --pad: var(--gap) 2px } \
             p { color: var(--main); padding: var(--pad); margin-left: calc(var(--gap) * 2) }",
        );
        assert_eq!(p.color, Color::rgb8(0, 128, 0));
        assert_eq!(p.padding, [4.0, 2.0, 4.0, 2.0].map(LengthPercentage::Px));
        assert_eq!(p.margin[LEFT], LengthPercentageAuto::Px(8.0));
        assert_eq!(
            p.custom_property("--pad"),
            Some(css::tokenize("4px 2px").as_slice())
        );
        assert_eq!(p.custom_property("--nope"), None);
        // Names are case-sensitive, values kept as written.
        let p = p_with(
            "<p>x</p>",
            "p { --Color: red; --color: #00f; color: var(--color) }",
        );
        assert_eq!(p.color, Color::rgb8(0, 0, 255));
        // An inline style's custom properties, and its var().
        let p = p_with(
            "<div style='--w: 30px'><p style='width: var(--w)'>x</p></div>",
            "",
        );
        assert_eq!(p.width, Size::Px(30.0));
        // `font-size` substitutes before the lengths that read it.
        let p = p_with(
            "<div style='font-size: 10px'><p>x</p></div>",
            "p { --size: 2em; font-size: var(--size); --w: 3em; width: var(--w) }",
        );
        assert_eq!(p.font_size_px, 20.0);
        assert_eq!(p.width, Size::Px(60.0));
        // Environment variables.
        let p = p_with(
            "<p>x</p>",
            "p { width: env(safe-area-inset-left, 5px); height: env(nope, 7px) }",
        );
        assert_eq!((p.width, p.height), (Size::Px(0.0), Size::Px(7.0)));
    }

    #[test]
    fn a_value_invalid_once_substituted_is_unset() {
        // Values 5's example: not the red the declaration beat, but the
        // initial transparent.
        let p = p_with(
            "<p>x</p>",
            ":root { --not-a-color: 20px } p { background-color: red } \
             p { background-color: var(--not-a-color) }",
        );
        assert_eq!(p.background_color, Color::TRANSPARENT);
        // An inherited property takes the parent's value.
        let p = p_with(
            "<div><p>x</p></div>",
            "div { color: #008000; font-size: 12px } \
             p { color: red; color: var(--missing); font-size: 30px; font-size: var(--missing) }",
        );
        assert_eq!(p.color, Color::rgb8(0, 128, 0));
        assert_eq!(p.font_size_px, 12.0);
        // Substitution is of tokens: `20` and `px` are two.
        let p = p_with(
            "<p>x</p>",
            "p { --gap: 20; width: 5px; width: var(--gap)px; height: calc(var(--gap) * 1px) }",
        );
        assert_eq!((p.width, p.height), (Size::Auto, Size::Px(20.0)));
        // A substitution function invalid by its grammar makes the
        // declaration invalid when it is parsed: the one before stands.
        let p = p_with(
            "<p>x</p>",
            "p { width: 5px; width: var(--a, !); height: 6px; height: var() }",
        );
        assert_eq!((p.width, p.height), (Size::Px(5.0), Size::Px(6.0)));
        // A CSS-wide keyword substituted is that keyword.
        let p = p_with(
            "<div style='color: #00f'><p>x</p></div>",
            "p { color: red; color: var(--nope, initial) }",
        );
        assert_eq!(p.color, Color::BLACK);
        let p = p_with(
            "<div style='width: 50px'><p>x</p></div>",
            "p { width: var(--nope, inherit) }",
        );
        assert_eq!(p.width, Size::Px(50.0));
        // A property the engine does not know is passed over.
        let p = p_with("<p>x</p>", "p { --c: red; bogus: var(--c) }");
        assert_eq!(p.color, Color::BLACK);
    }

    #[test]
    fn a_shorthand_substitutes_whole_and_its_longhands_after_it_win() {
        let p = p_with(
            "<p>x</p>",
            "p { --m: 1px 2px; margin: var(--m); --b: 3px solid; border: var(--b) #00f }",
        );
        assert_eq!(p.margin, [1.0, 2.0, 1.0, 2.0].map(LengthPercentageAuto::Px));
        assert_eq!(p.border_width, [3.0; 4]);
        assert_eq!(p.border_style, [BorderStyle::Solid; 4]);
        assert_eq!(p.border_color, [Color::rgb8(0, 0, 255); 4]);
        // Invalid, it leaves every longhand unset; a longhand written
        // after it still wins.
        let p = p_with(
            "<p>x</p>",
            "p { margin: 9px; margin: var(--bad); margin-left: 5px }",
        );
        assert_eq!(p.margin, [0.0, 0.0, 0.0, 5.0].map(LengthPercentageAuto::Px));
        // Two values substituted with no white space between them are
        // two, as the reference browser has them.
        let p = p_with(
            "<p>x</p>",
            "p { --a: 1px; --b: 2px; margin: var(--a)var(--b) }",
        );
        assert_eq!(p.margin, [1.0, 2.0, 1.0, 2.0].map(LengthPercentageAuto::Px));
        // As are two values a comment parts, and a hash after a keyword.
        let p = p_with(
            "<p>x</p>",
            "p { margin: 1px/**/2px; border: 1px solid#00f }",
        );
        assert_eq!(p.margin, [1.0, 2.0, 1.0, 2.0].map(LengthPercentageAuto::Px));
        assert_eq!(p.border_color, [Color::rgb8(0, 0, 255); 4]);
        // An alias substitutes as its property.
        let p = p_with(
            "<p>x</p>",
            "p { --o: 2; -webkit-order: var(--o); --g: 1fr 2fr; display: grid; \
             grid-template-columns: var(--g) }",
        );
        assert_eq!(p.order, 2);
        assert_eq!(
            p.grid().template_columns.as_ref().map(|t| t.sections.len()),
            Some(2)
        );
    }

    #[test]
    fn custom_properties_apply_to_every_pseudo_element() {
        // Variables 1's tests of ::first-line and ::first-letter: the
        // custom properties apply, and the properties they substitute
        // into as the pseudo-element takes them.
        let sheets = [sheet(
            "p::first-line { --c: #00f; color: var(--c); --pos: absolute; position: var(--pos) } \
             p::first-letter { --m: 7px; margin-left: var(--m) } \
             p { --t: 'hi' } p::before { content: var(--t) ' there' }",
        )];
        let html = "<p>x</p>";
        let line = pseudo_of(html, &sheets, "p", PseudoElement::FirstLine).expect("styled");
        assert_eq!(line.color, Color::rgb8(0, 0, 255));
        assert_eq!(line.position, Position::Static);
        assert_eq!(
            line.custom_property("--c"),
            Some(css::tokenize("#00f").as_slice())
        );
        let letter = pseudo_of(html, &sheets, "p", PseudoElement::FirstLetter).expect("styled");
        assert_eq!(letter.margin[LEFT], LengthPercentageAuto::Px(7.0));
        let before = pseudo_of(html, &sheets, "p", PseudoElement::Before).expect("styled");
        assert_eq!(
            before.content,
            Content::Items(Arc::from([
                ContentItem::Text("hi".into()),
                ContentItem::Text(" there".into())
            ]))
        );
    }

    #[test]
    fn custom_properties_cascade_as_declarations_do() {
        // Importance, layers, and the last valid declaration of a name.
        let p = p_with(
            "<p id=p>x</p>",
            "@layer base { p { --c: #f00 !important } } \
             #p { --c: #0f0 } p { --c: #00f !important; --d: 1px; --d: !; color: var(--c); \
             width: var(--d) }",
        );
        assert_eq!(p.color, Color::rgb8(255, 0, 0));
        assert_eq!(p.width, Size::Px(1.0));
        // A cycle makes its properties guaranteed-invalid, a fallback in
        // it notwithstanding, and their users take their own fallbacks.
        let p = p_with(
            "<p>x</p>",
            "p { --a: var(--b, 1px); --b: var(--a, 2px); width: var(--a, 3px); \
             height: var(--b) }",
        );
        assert_eq!((p.width, p.height), (Size::Px(3.0), Size::Auto));
        // `@supports` takes custom properties.
        let p = p_with(
            "<p>x</p>",
            "@supports (--a: b) and (color: var(--x)) { p { color: #0f0 } }",
        );
        assert_eq!(p.color, Color::rgb8(0, 255, 0));
    }

    #[test]
    fn a_set_on_every_element_is_computed_once() {
        // Declared on `*`, the same declarations are the parent's, and
        // the parent's map is shared, not computed again.
        let html = "<div><section><p>x</p></section></div>";
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let sheets = [sheet("* { --a: 1; --b: var(--a) var(--a) } p { --c: 2 }")];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let custom = |tag| {
            styles
                .get(first(&document, tag))
                .unwrap()
                .custom
                .clone()
                .unwrap()
        };
        assert!(Arc::ptr_eq(&custom("div"), &custom("section")));
        assert!(Arc::ptr_eq(&custom("html"), &custom("div")));
        assert!(!Arc::ptr_eq(&custom("section"), &custom("p")));
        assert_eq!(custom("p").len(), 3);
        // An element that declares none shares its parent's too.
        let document =
            Document::from_node(&html::parse_document("<div><p>x</p></div>").expect("never fails"));
        let sheets = [sheet("div { --a: 1 }")];
        let styles = cascade_document(&document, &sheets, &CascadeContext::default());
        let custom = |tag| {
            styles
                .get(first(&document, tag))
                .unwrap()
                .custom
                .clone()
                .unwrap()
        };
        assert!(Arc::ptr_eq(&custom("div"), &custom("p")));
        assert_eq!(styles.get(first(&document, "html")).unwrap().custom, None);
    }

    #[test]
    fn the_bytes_count_a_style_elements_share_once() {
        // M35.5: a hundred paragraphs of one style hold it once.
        let bytes = |count: usize| {
            let html = "<p>x</p>".repeat(count);
            let document = Document::from_node(&html::parse_document(&html).expect("never fails"));
            let sheets = [sheet("p { color: red }")];
            cascade_document(&document, &sheets, &CascadeContext::default()).heap_bytes()
        };
        let more = bytes(100) - bytes(2);
        assert!(
            more < 98 * std::mem::size_of::<ComputedStyle>() / 4,
            "{more}"
        );
    }

    #[test]
    fn attr_substitutes_into_any_property() {
        // M21.5: by its type, ASCII case-insensitively by name.
        let p = p_with(
            "<p data-w=10 data-c=#00f data-len='calc(1px + 3px)' TITLE=x>x</p>",
            "p { width: attr(DATA-W px); color: attr(data-c type(<color>), red); \
             height: attr(data-len type(<length>)); margin-left: attr(nope px, 3px); \
             --t: attr(title) }",
        );
        assert_eq!((&p.width, &p.height), (&Size::Px(10.0), &Size::Px(4.0)));
        assert_eq!(p.color, Color::rgb8(0, 0, 255));
        assert_eq!(p.margin[LEFT], LengthPercentageAuto::Px(3.0));
        assert_eq!(
            p.custom_property("--t"),
            Some(css::tokenize("'x'").as_slice())
        );
        // A value its property does not take: invalid at computed-value
        // time. A first argument that is no name and type: invalid where
        // it is parsed, the declaration before it standing.
        let p = p_with(
            "<p data-w=abc>x</p>",
            "p { width: 5px; width: attr(data-w px); height: 6px; height: attr(data-w &, 3px) }",
        );
        assert_eq!((p.width, p.height), (Size::Auto, Size::Px(6.0)));
        // A pseudo-element reads its originating element's attributes.
        let sheets = [sheet(
            "p::first-letter { margin-left: attr(data-m px) } \
             p::before { content: attr(data-t) ' ' attr(data-m) }",
        )];
        let html = "<p data-m=7 data-t=hi>x</p>";
        let letter = pseudo_of(html, &sheets, "p", PseudoElement::FirstLetter).expect("styled");
        assert_eq!(letter.margin[LEFT], LengthPercentageAuto::Px(7.0));
        let before = pseudo_of(html, &sheets, "p", PseudoElement::Before).expect("styled");
        assert_eq!(
            before.content,
            Content::Items(Arc::from([
                ContentItem::Text("hi".into()),
                ContentItem::Text(" ".into()),
                ContentItem::Text("7".into()),
            ]))
        );
        // Through a custom property, each element its own.
        let p = p_with(
            "<div data-a=1><p data-a=2>x</p></div>",
            "* { --a: attr(data-a type(<number>), 0) } p { width: calc(var(--a) * 1px) }",
        );
        assert_eq!(p.width, Size::Px(2.0));
    }

    #[test]
    fn the_counter_properties_parse() {
        // Lists 3 §4 (M21.4).
        use crate::computed::{CounterChange, CounterReset};
        let reset = |name: &str, reversed: bool, value: Option<i32>| CounterReset {
            name: Arc::from(name),
            reversed,
            value,
        };
        let change = |name: &str, value: i32| CounterChange {
            name: Arc::from(name),
            value,
        };
        let p = p_with(
            "<p>x</p>",
            "p { counter-reset: a b 3 reversed(c) reversed( d ) -2 a 7 E calc(2.5); \
             counter-increment: a b -1 a; counter-set: x y 99999999999 }",
        );
        let lists = p.lists();
        assert_eq!(
            lists.counter_reset,
            [
                reset("b", false, Some(3)),
                reset("c", true, None),
                reset("d", true, Some(-2)),
                reset("a", false, Some(7)),
                reset("E", false, Some(3)),
            ]
        );
        assert_eq!(
            lists.counter_increment,
            [change("a", 1), change("b", -1), change("a", 1)]
        );
        assert_eq!(lists.counter_set, [change("x", 0), change("y", i32::MAX)]);
        // `none`, and the initial values, keep no copy.
        let p = p_with(
            "<p>x</p>",
            "p { counter-reset: a } p { counter-reset: NONE; counter-increment: none }",
        );
        assert_eq!(p.lists, None);
        // Not inherited; `inherit` takes the parent's.
        let p = p_with(
            "<div><p>x</p></div>",
            "div { counter-increment: a 2 } p { counter-set: inherit }",
        );
        assert_eq!(p.lists().counter_increment, []);
        assert_eq!(p.lists().counter_set, []);
        let p = p_with(
            "<div><p>x</p></div>",
            "div { counter-increment: a 2 } p { counter-increment: inherit }",
        );
        assert_eq!(p.lists().counter_increment, [change("a", 2)]);
        // What is not a list of names and integers drops the declaration.
        for bad in [
            "none a",
            "a none",
            "1",
            "a 1 2",
            "'a'",
            "inherit a",
            "reversed(a)",
            "a 1.5",
            "a 1px",
            "default",
        ] {
            let p = p_with(
                "<p>x</p>",
                &format!("p {{ counter-increment: z 5 }} p {{ counter-increment: {bad} }}"),
            );
            assert_eq!(p.lists().counter_increment, [change("z", 5)], "{bad}");
        }
        for bad in [
            "reversed()",
            "reversed(none)",
            "reversed(a b)",
            "reversed(1)",
            "none none",
        ] {
            let p = p_with(
                "<p>x</p>",
                &format!("p {{ counter-reset: z }} p {{ counter-reset: {bad} }}"),
            );
            assert_eq!(
                p.lists().counter_reset,
                [reset("z", false, Some(0))],
                "{bad}"
            );
        }
    }

    #[test]
    fn custom_properties_are_supported() {
        for (name, value) in [
            ("--a", ""),
            ("--a", "anything { at all }"),
            ("--a", "initial"),
            ("--a", "revert"),
            ("--a", "revert-layer"),
            ("margin", "revert"),
            ("--A", "var(--b, var(--c))"),
            ("color", "var(--a)"),
            ("margin", "var(--a) 1px"),
            ("-webkit-order", "var(--a)"),
            ("color", "env(safe-area-inset-top, red)"),
        ] {
            assert!(supports(name, &css::tokenize(value)), "{name}: {value}");
        }
        for (name, value) in [
            ("--", "a"),
            ("--a", "!"),
            ("--a", ")"),
            ("bogus", "revert"),
            ("color", "var(--a, !)"),
            ("color", "var()"),
            ("bogus", "var(--a)"),
        ] {
            assert!(!supports(name, &css::tokenize(value)), "{name}: {value}");
        }
    }

    #[test]
    fn display_contents_and_the_elements_it_makes_none_of() {
        let sheets = [sheet(
            "html, div, img, input, svg, span { display: contents }",
        )];
        let html = "<div id=d><img><input><svg><g></g></svg><span><b>x</b></span></div>";
        let display = |selector: &str| style_of(html, &sheets, selector).display;
        assert_eq!(display("html"), Display::Block, "the root's is block");
        assert_eq!(display("#d"), Display::Contents);
        assert_eq!(display("span"), Display::Contents);
        assert_eq!(display("img"), Display::None);
        assert_eq!(display("input"), Display::None);
        assert_eq!(display("svg"), Display::None, "an outer svg");
        // Its children are a grid's items where its parent is a grid.
        let sheets = [sheet("#g { display: grid } span { display: contents }")];
        let html = "<div id=g><span><b>x</b></span></div><p><span><i>y</i></span></p>";
        assert_eq!(style_of(html, &sheets, "b").display, Display::Block);
        assert_eq!(style_of(html, &sheets, "i").display, Display::Inline);
        assert_eq!(
            style_of(html, &sheets, "#g span").display,
            Display::Contents
        );
    }

    #[test]
    fn math_is_mathmls_and_its_elements_take_their_layouts() {
        use crate::computed::MathElement;
        let sheets = [sheet(
            "div, mrow { display: block math } p { display: math }",
        )];
        let html = "<div></div><p></p><math><mrow></mrow>\
                    <msub><mi>x</mi><mn>2</mn></msub>\
                    <mfrac linethickness=2px><mn>1</mn><mn>3</mn></mfrac>\
                    <mspace width=10px height=1em depth=2px></mspace>\
                    <semantics><mi>a</mi><annotation>b</annotation><mi>c</mi></semantics></math>";
        let of = |selector: &str| style_of(html, &sheets, selector);
        assert_eq!(of("div").display, Display::Block);
        assert_eq!(of("p").display, Display::Inline);
        assert_eq!(of("mrow").display, Display::BlockMath);
        assert_eq!(of("math").display, Display::Math);
        assert_eq!(of("msub").text().math, MathElement::Sub);
        assert_eq!(of("mrow").text().math, MathElement::Row);
        assert_eq!(of("div").text().math, MathElement::None);
        assert_eq!(
            of("mfrac").text().math,
            MathElement::Fraction {
                thickness: Some(2.0)
            }
        );
        let space = of("mspace");
        assert_eq!(
            space.text().math,
            MathElement::Space {
                height: 16.0,
                depth: 2.0
            }
        );
        assert_eq!(space.width, Size::Px(10.0));
        // A script inherits a smaller font, compact; its base does not.
        let script = of("msub > mn");
        assert!((script.font_size_px - 16.0 * 0.71).abs() < 0.01);
        assert_eq!(
            script.text().math_style,
            crate::computed::MathStyle::Compact
        );
        assert_eq!(of("msub > mi").font_size_px, 16.0);
        // In a formula set inline, a fraction's parts too.
        assert!((of("mfrac > mn").font_size_px - 16.0 * 0.71).abs() < 0.01);
        // `semantics` shows its first child alone.
        assert_eq!(of("semantics > mi").display, Display::BlockMath);
        assert_eq!(
            of("semantics > mi + annotation + mi").display,
            Display::None
        );
    }

    #[test]
    fn ruby_takes_its_displays_and_its_properties() {
        let html = "<ruby>a<rt>b</rt><rp>(</rp></ruby>";
        let sheets = [sheet("ruby { ruby-position: under; ruby-align: start }")];
        let of = |selector: &str| style_of(html, &sheets, selector);
        assert_eq!(of("ruby").display, Display::Ruby);
        assert_eq!(of("rp").display, Display::None);
        let rt = of("rt");
        assert_eq!(rt.display, Display::RubyText);
        assert_eq!(
            rt.text().ruby_position,
            crate::computed::RubyPosition::Under
        );
        assert_eq!(rt.text().ruby_align, crate::computed::RubyAlign::Start);
        assert_eq!(rt.font_size_px, 8.0);
        let p = p_with("<p>x</p>", "p { display: ruby; float: left }");
        assert_eq!(p.display, Display::BlockRuby, "blockified");
        let p = p_with("<p>x</p>", "p { display: ruby-text; position: absolute }");
        assert_eq!(p.display, Display::Block);
        let p = p_with("<p>x</p>", "p { display: run-in }");
        assert_eq!(p.display, Display::Block, "run-in is not taken");
    }

    #[test]
    fn list_item_takes_its_block_forms() {
        // css-display-3 §2.4, M21.6.
        for value in [
            "list-item",
            "LIST-ITEM",
            "block list-item",
            "list-item block",
            "flow list-item",
            "list-item flow block",
            "flow-root list-item",
        ] {
            let p = p_with("<p>x</p>", &format!("p {{ display: {value} }}"));
            assert_eq!(p.display, Display::ListItem, "{value}");
        }
        for value in [
            "list-item list-item",
            "block flow",
            "list-item 2",
            "list-item table",
        ] {
            let p = p_with("<p>x</p>", &format!("p {{ display: {value} }}"));
            assert_eq!(p.display, Display::Block, "{value}");
        }
        // Its inline forms (D77), a flow root among them.
        for value in ["inline list-item", "list-item flow inline"] {
            let p = p_with("<p>x</p>", &format!("p {{ display: {value} }}"));
            assert_eq!((p.display, p.flow_root), (Display::InlineListItem, false));
        }
        let p = p_with("<p>x</p>", "p { display: list-item inline flow-root }");
        assert_eq!((p.display, p.flow_root), (Display::InlineListItem, true));
        // A floated list item stays one.
        let p = p_with("<p>x</p>", "p { display: list-item; float: left }");
        assert_eq!(p.display, Display::ListItem);
    }

    #[test]
    fn the_list_properties_parse_and_inherit() {
        use crate::computed::{ListStylePosition, ListStyleType};
        let style = |name: &str| ListStyleType::Style(Arc::from(name));
        let of = |declarations: &str| {
            let p = p_with("<div><p>x</p></div>", &format!("div {{ {declarations} }}"));
            let lists = p.lists();
            (
                lists.list_style_type.clone(),
                lists.list_style_position,
                lists.list_style_image.clone(),
            )
        };
        let url = |u: &str| {
            Some(crate::computed::Image::Url {
                url: Arc::from(u),
                resolution: 1.0,
            })
        };
        let outside = ListStylePosition::Outside;
        let inside = ListStylePosition::Inside;
        assert_eq!(of(""), (style("disc"), outside, None));
        assert_eq!(
            of("list-style-type: Upper-Roman"),
            (style("upper-roman"), outside, None)
        );
        assert_eq!(
            of("list-style-type: My-Own"),
            (style("My-Own"), outside, None)
        );
        assert_eq!(
            of("list-style-type: '-> '"),
            (ListStyleType::String(Arc::from("-> ")), outside, None)
        );
        assert_eq!(
            of("list-style-type: none; list-style-position: inside"),
            (ListStyleType::None, inside, None)
        );
        assert_eq!(
            of("list-style-image: url(a.png)"),
            (style("disc"), outside, url("a.png"))
        );
        // The shorthand, and its `none` (§3.6).
        assert_eq!(
            of("list-style: square inside"),
            (style("square"), inside, None)
        );
        assert_eq!(of("list-style: none"), (ListStyleType::None, outside, None));
        assert_eq!(
            of("list-style: none url(b)"),
            (ListStyleType::None, outside, url("b"))
        );
        assert_eq!(
            of("list-style: none decimal"),
            (style("decimal"), outside, None)
        );
        assert_eq!(
            of("list-style: none none inside"),
            (ListStyleType::None, inside, None)
        );
        assert_eq!(
            of("list-style-type: square; list-style: inside"),
            (style("disc"), inside, None)
        );
        for bad in [
            "none none none",
            "none decimal url(c)",
            "inside outside",
            "decimal square",
            "url(a) url(b)",
            "symbols(cyclic '*')",
            "1",
        ] {
            assert_eq!(
                of(&format!("list-style: circle; list-style: {bad}")),
                (style("circle"), outside, None),
                "{bad}"
            );
        }
        // Inherited, all three, and not the counter properties.
        let p = p_with(
            "<div><p>x</p></div>",
            "div { list-style: circle inside url(x); counter-reset: c }",
        );
        assert_eq!(p.lists().list_style_type, style("circle"));
        assert_eq!(p.lists().list_style_position, inside);
        assert!(p.lists().counter_reset.is_empty());
    }

    #[test]
    fn a_list_items_marker_takes_the_properties_that_apply_to_it() {
        let sheets = [sheet(
            "li::marker { color: #f00; font-size: 20px; margin-left: 9px; content: 'x';
                 text-transform: uppercase; display: block; --m: 1; line-height: 30px;
                 word-break: break-all }
             p::marker { color: #f00 }",
        )];
        let marker = pseudo_of("<ol><li>a</li></ol>", &sheets, "li", PseudoElement::Marker)
            .expect("a list item has a marker");
        assert_eq!(marker.color, Color::rgb8(255, 0, 0));
        assert_eq!(marker.font_size_px, 20.0);
        assert_eq!(marker.content, items(&[text("x")]));
        assert_eq!(marker.text_transform, TextTransform::Uppercase);
        assert!(marker.custom_property("--m").is_some());
        assert_eq!(
            marker.font().line_height,
            crate::computed::LineHeight::Px(30.0)
        );
        assert_eq!(marker.word_break, crate::computed::WordBreak::BreakAll);
        // Not the box's properties; an outside marker is an inline block
        // whatever its rule says.
        assert_eq!(marker.margin[LEFT], LengthPercentageAuto::Px(0.0));
        assert_eq!(marker.display, Display::InlineBlock);
        // Unstyled, it inherits the list item's, with the user agent's
        // rule; its `content` stays `normal`.
        let marker = pseudo_of(
            "<ul><li style='color: #00f; text-transform: uppercase'>a</li></ul>",
            &[],
            "li",
            PseudoElement::Marker,
        )
        .expect("a list item has a marker");
        assert_eq!(marker.color, Color::rgb8(0, 0, 255));
        assert_eq!(marker.content, Content::Normal);
        assert_eq!(marker.text_transform, TextTransform::None);
        assert_eq!(marker.white_space, WhiteSpace::PRE);
        // An isolate of tabular figures, not indented nor aligned as its
        // list item's text; inline when inside.
        let marker = pseudo_of(
            "<ol style='text-indent: 5px; text-align: center'>\
             <li style='list-style-position: inside'>a</li></ol>",
            &[],
            "li",
            PseudoElement::Marker,
        )
        .expect("a list item has a marker");
        assert_eq!(
            marker.text().unicode_bidi,
            crate::computed::UnicodeBidi::Isolate
        );
        assert_eq!(marker.display, Display::Inline);
        assert_eq!(marker.text_align, TextAlign::Start);
        assert_eq!(
            marker.text().text_indent,
            crate::computed::TextIndent::default()
        );
        // What is no list item has none.
        assert_eq!(
            pseudo_of("<p>a</p>", &sheets, "p", PseudoElement::Marker),
            None
        );
    }

    #[test]
    fn the_ua_sheet_numbers_and_bullets_lists_as_the_html_standard_does() {
        use crate::computed::ListStyleType;
        let html = "<ol id=o><li id=a>a<ul id=u><li id=b>b<ol><li>c<menu><li id=d>d\
                    <ul><li id=e>e</ul></menu></ol></ul></ol><dl><dd><dl id=dl></dl></dl>";
        let of = |selector: &str| style_of(html, &[], selector);
        let kind = |selector: &str| of(selector).lists().list_style_type.clone();
        let style = |name: &str| ListStyleType::Style(Arc::from(name));
        assert_eq!(of("#a").display, Display::ListItem);
        assert_eq!(kind("#a"), style("decimal"));
        assert_eq!(kind("#b"), style("circle"));
        assert_eq!(kind("#d"), style("square"));
        assert_eq!(kind("#e"), style("square"));
        let reset = |selector: &str| {
            of(selector)
                .lists()
                .counter_reset
                .iter()
                .map(|r| (r.name.to_string(), r.reversed, r.value))
                .collect::<Vec<_>>()
        };
        assert_eq!(reset("#o"), [("list-item".to_string(), false, Some(0))]);
        assert_eq!(reset("#u"), [("list-item".to_string(), false, Some(0))]);
        // A nested list has no margins above and below.
        assert_eq!(of("#o").margin[TOP], LengthPercentageAuto::Px(16.0));
        assert_eq!(of("#u").margin[TOP], LengthPercentageAuto::Px(0.0));
        assert_eq!(of("#dl").margin[BOTTOM], LengthPercentageAuto::Px(0.0));
    }

    #[test]
    fn an_img_width_and_height_are_presentational_hints() {
        use crate::computed::{AspectRatio, Ratio};
        let img = |html: &str, css: &str| style_of(html, &[sheet(css)], "img");
        let s = img("<img width=300 height=150>", "");
        assert_eq!(
            (s.width.clone(), s.height.clone()),
            (Size::Px(300.0), Size::Px(150.0))
        );
        assert_eq!(
            s.image().aspect_ratio,
            AspectRatio::AutoOr(Ratio {
                width: 300.0,
                height: 150.0
            })
        );
        let s = img("<img width=50% height=20>", "");
        assert_eq!(s.width, Size::Percent(50.0));
        assert_eq!(s.image().aspect_ratio, AspectRatio::Auto);
        // An author rule overrides a hint, however unspecific, and the
        // ratio stays to size the other side by.
        let s = img("<img width=300 height=150>", "* { height: auto }");
        assert_eq!(s.height, Size::Auto);
        assert!(matches!(s.image().aspect_ratio, AspectRatio::AutoOr(_)));
        // An image button's are hints, a text field's none.
        let s = style_of("<input type=image width=30 hspace=2%>", &[], "input");
        assert_eq!(s.width, Size::Px(30.0));
        assert_eq!(s.margin[3], LengthPercentageAuto::Percent(2.0));
        let s = style_of("<input width=30>", &[], "input");
        assert_eq!(s.width, Size::Auto);
        // A video's are hints too (HTML section 15.4.3), a div's none.
        let s = style_of("<video width=300></video>", &[], "video");
        assert_eq!(s.width, Size::Px(300.0));
        let s = style_of("<div width=300></div>", &[], "div");
        assert_eq!(s.width, Size::Auto);
    }

    #[test]
    fn a_form_in_a_tables_parts_is_hidden_in_an_html_document_alone() {
        // HTML section 15.3.8: the user agent's `!important`, over an
        // author's.
        let css = "form { display: block !important }";
        for parent in ["table", "thead", "tbody", "tfoot", "tr"] {
            let mut document = Document::from_node(&html::parse_document("").expect("never fails"));
            let body = document
                .subtree(document.root())
                .find(|&n| document.tag(n) == Some("body"))
                .expect("a body");
            let table = document.create_element(parent).expect("a valid name");
            let form = document.create_element("form").expect("a valid name");
            document.append_child(body, table).expect("appended");
            document.append_child(table, form).expect("appended");
            let styles = cascade_document(&document, &[sheet(css)], &CascadeContext::default());
            assert_eq!(
                styles.get(form).map(|s| s.display),
                Some(Display::None),
                "{parent}"
            );
        }
        let s = style_of("<div><form></form></div>", &[], "form");
        assert_eq!(s.display, Display::Block);
        let xml = html::xml::parse_xml_text(
            "<table xmlns='http://www.w3.org/1999/xhtml'><form id='f'/></table>",
        )
        .expect("well-formed");
        let styles = cascade_document(&xml, &[], &CascadeContext::default());
        let form = xml
            .subtree(xml.root())
            .find(|&n| xml.tag(n) == Some("form"))
            .expect("a form");
        assert_eq!(styles.get(form).map(|s| s.display), Some(Display::Block));
    }

    #[test]
    fn unicode_bidi_and_appearance_compute_and_do_not_inherit() {
        use crate::computed::{Appearance, UnicodeBidi};
        // The UA sheet's isolates and overrides (HTML section 15.3.5).
        let s = style_of("<p><span>x</span></p>", &[], "p");
        assert_eq!(s.text().unicode_bidi, UnicodeBidi::Isolate);
        let s = style_of("<p><span>x</span></p>", &[], "span");
        assert_eq!(s.text().unicode_bidi, UnicodeBidi::Normal);
        let s = style_of("<bdo dir=rtl>x</bdo>", &[], "bdo");
        assert_eq!(s.text().unicode_bidi, UnicodeBidi::IsolateOverride);
        let css = "span { unicode-bidi: Bidi-Override } i { unicode-bidi: inherit }";
        let s = style_of("<span><i>x</i></span>", &[sheet(css)], "i");
        assert_eq!(s.text().unicode_bidi, UnicodeBidi::BidiOverride);
        let s = style_of("<span><b>x</b></span>", &[sheet(css)], "b");
        assert_eq!(s.text().unicode_bidi, UnicodeBidi::Normal);
        // Taking a first line's inherited properties keeps it, as it
        // keeps every property not inherited.
        let p = style_of("<p>x</p>", &[], "p");
        let line = style_of("<p style='text-indent: 3px'>x</p>", &[], "p");
        let on_line = p.with_inherited_from(&line);
        assert_eq!(on_line.text().unicode_bidi, UnicodeBidi::Isolate);
        assert_eq!(
            on_line.text().text_indent.length,
            crate::computed::LengthPercentage::Px(3.0)
        );
        // A control's native look, and its `-webkit-` alias.
        let s = style_of("<input>", &[], "input");
        assert_eq!(s.effects().appearance, Appearance::Auto);
        let s = style_of("<input type=hidden>", &[], "input");
        assert_eq!(s.effects().appearance, Appearance::None);
        let css = "button { -webkit-appearance: menulist-button }";
        let s = style_of("<button></button>", &[sheet(css)], "button");
        assert_eq!(s.effects().appearance, Appearance::MenulistButton);
        let css = "button { appearance: push-button }";
        let s = style_of("<button></button>", &[sheet(css)], "button");
        assert_eq!(s.effects().appearance, Appearance::Auto);
    }

    #[test]
    fn the_image_properties_cascade_and_two_inherit() {
        use crate::computed::{ImageOrientation, ImageRendering, ObjectFit};
        let css = "div { object-fit: cover; object-position: right top; image-rendering: pixelated;
                   image-orientation: none; aspect-ratio: 2 }";
        let div = style_of("<div><p>x</p></div>", &[sheet(css)], "div");
        assert_eq!(div.image().object_fit, ObjectFit::Cover);
        assert_eq!(
            div.image().object_position.x,
            LengthPercentage::Percent(100.0)
        );
        assert_eq!(
            div.image().object_position.y,
            LengthPercentage::Percent(0.0)
        );
        let p = style_of("<div><p>x</p></div>", &[sheet(css)], "p");
        assert_eq!(p.image().image_rendering, ImageRendering::Pixelated);
        assert_eq!(p.image().image_orientation, ImageOrientation::None);
        assert_eq!(p.image().object_fit, ObjectFit::Fill);
        assert_eq!(p.image().aspect_ratio, crate::computed::AspectRatio::Auto);
        let css = "div { object-fit: cover; image-rendering: pixelated }
                   p { object-fit: inherit; image-rendering: initial }";
        let p = style_of("<div><p>x</p></div>", &[sheet(css)], "p");
        assert_eq!(p.image().object_fit, ObjectFit::Cover);
        assert_eq!(p.image().image_rendering, ImageRendering::Auto);
        // Nothing set, nothing allocated.
        assert!(style_of("<p>x</p>", &[], "p").image.is_none());
    }

    #[test]
    fn svgs_properties_cascade_from_presentation_attributes_and_sheets() {
        // The reference browser's computed values (M32, the design
        // note's probes).
        use crate::computed::TransformFunction;
        use crate::serialize::computed_value;
        let html = r##"<div fill="red" id="d"></div><svg id="s" width="200" height="100">
  <rect id="r1" width="10" height="10"/>
  <rect id="r2" fill="red"/>
  <rect id="r2b" fill="red"/>
  <rect id="r3" fill="red" style="fill: blue"/>
  <rect id="r4" x="5" y="6" width="7" height="8" rx="2" fill="#00f" stroke="red"
    stroke-width="3" opacity="0.5" fill-opacity=".25" transform="translate(3 4)"/>
  <circle id="ci" cx="10" cy="10%" r="5" fill="currentColor" color="orange" x="9"/>
  <g id="gr" fill="purple" style="x: 5px"><rect id="r5"/><rect id="r9" fill="inherit"/>
    <rect id="r13" fill="blue" style="fill: unset"/></g>
  <text id="t" x="10" y="20" font-size="12" text-anchor="middle"
    dominant-baseline="central">Hi</text>
  <text id="t2" font-size="calc(2 * 6px)" paint-order="stroke">Hi</text>
  <rect id="r12" fill="black"/>
  <rect id="r6" fill="url(#nope) yellow" stroke="url(#nope)"/>
  <rect id="r7" fill="bogus" stroke="red !important" stroke-width="-1"/>
  <rect id="r8" stroke-dasharray="1, 2 3" stroke-width="2em" opacity="2"
    fill-opacity="-1" stroke-linejoin="miter-clip"/>
  <rect id="r10" fill="red;stroke:blue" transform="scale(2) foo"
    stroke-miterlimit="0.5" paint-order="markers stroke fill"/>
  <rect id="r11" fill="var(--c)" style="--c: green" width="-5" rx="auto"/>
  <stop id="st" stop-color="blue" stop-opacity="2"/>
</svg>"##;
        let css = "body { color: rgb(10, 20, 30) } #r2b { fill: green }
                   #gr { stroke-width: 4 } #r9 { stroke-width: initial }";
        let of = |id: &str| style_of(html, &[sheet(css)], &format!("#{id}"));
        let value = |id: &str, name: &str| computed_value(&of(id), name).unwrap();
        let r1 = of("r1");
        for (name, want) in [
            ("fill", "rgb(0, 0, 0)"),
            ("stroke", "none"),
            ("stroke-width", "1px"),
            ("fill-opacity", "1"),
            ("fill-rule", "nonzero"),
            ("stroke-linecap", "butt"),
            ("stroke-linejoin", "miter"),
            ("stroke-miterlimit", "4"),
            ("stroke-dasharray", "none"),
            ("stroke-dashoffset", "0px"),
            ("x", "0px"),
            ("rx", "auto"),
            ("paint-order", "normal"),
            ("width", "10px"),
        ] {
            assert_eq!(computed_value(&r1, name).unwrap(), want, "{name}");
        }
        assert!(r1.svg.is_none(), "nothing set, nothing kept");
        // The lowest of the author's: any rule wins, `style` over both.
        assert_eq!(value("r2", "fill"), "rgb(255, 0, 0)");
        assert_eq!(value("r2b", "fill"), "rgb(0, 128, 0)");
        assert_eq!(value("r3", "fill"), "rgb(0, 0, 255)");
        let r4 = of("r4");
        for (name, want) in [
            ("fill", "rgb(0, 0, 255)"),
            ("stroke", "rgb(255, 0, 0)"),
            ("stroke-width", "3px"),
            ("opacity", "0.5"),
            ("fill-opacity", "0.25"),
            ("x", "5px"),
            ("y", "6px"),
            ("width", "7px"),
            ("rx", "2px"),
            ("ry", "auto"),
        ] {
            assert_eq!(computed_value(&r4, name).unwrap(), want, "{name}");
        }
        assert_eq!(
            r4.effects().transform,
            vec![TransformFunction::Matrix([1.0, 0.0, 0.0, 1.0, 3.0, 4.0])]
        );
        // `currentcolor` is the element's colour, from its own attribute
        // too; the geometry is the element's own.
        assert_eq!(value("ci", "fill"), "rgb(255, 165, 0)");
        assert_eq!(value("ci", "cy"), "10%");
        assert_eq!(value("ci", "r"), "5px");
        assert_eq!(value("ci", "x"), "0px");
        // Inherited, `inherit` too; a CSS-wide keyword in a sheet.
        assert_eq!(value("r5", "fill"), "rgb(128, 0, 128)");
        assert_eq!(value("r5", "stroke-width"), "4px");
        assert_eq!(value("r9", "fill"), "rgb(128, 0, 128)");
        assert_eq!(value("r9", "stroke-width"), "1px");
        assert_eq!(value("r13", "fill"), "rgb(128, 0, 128)", "unset inherits");
        // What a child does not inherit is not its parent's.
        assert_eq!(value("gr", "x"), "5px");
        assert_eq!(value("r5", "x"), "0px");
        // A value at its initial value keeps nothing.
        assert!(of("r12").svg.is_none());
        let t2 = of("t2");
        assert_eq!(t2.font_size_px, 12.0, "a number in a function is no pixels");
        assert_eq!(computed_value(&t2, "paint-order").unwrap(), "stroke");
        let t = of("t");
        assert_eq!(t.font_size_px, 12.0);
        assert_eq!(computed_value(&t, "text-anchor").unwrap(), "middle");
        assert_eq!(computed_value(&t, "dominant-baseline").unwrap(), "central");
        assert_eq!(
            computed_value(&t, "x").unwrap(),
            "0px",
            "a text's x is a list"
        );
        assert_eq!(computed_value(&t, "display").unwrap(), "block");
        assert_eq!(value("r6", "fill"), "url(\"#nope\") rgb(255, 255, 0)");
        assert_eq!(value("r6", "stroke"), "url(\"#nope\")");
        // A value the property does not take is ignored.
        assert_eq!(value("r7", "fill"), "rgb(0, 0, 0)");
        assert_eq!(value("r7", "stroke"), "none");
        assert_eq!(value("r7", "stroke-width"), "1px");
        assert_eq!(value("r8", "stroke-dasharray"), "1px, 2px, 3px");
        assert_eq!(value("r8", "stroke-width"), "32px");
        assert_eq!(value("r8", "opacity"), "1");
        assert_eq!(value("r8", "fill-opacity"), "0");
        assert_eq!(value("r8", "stroke-linejoin"), "miter");
        let r10 = of("r10");
        assert_eq!(computed_value(&r10, "fill").unwrap(), "rgb(0, 0, 0)");
        assert_eq!(computed_value(&r10, "stroke").unwrap(), "none");
        assert!(r10.effects().transform.is_empty());
        assert_eq!(computed_value(&r10, "stroke-miterlimit").unwrap(), "0.5");
        assert_eq!(
            computed_value(&r10, "paint-order").unwrap(),
            "markers stroke"
        );
        let r11 = of("r11");
        assert_eq!(computed_value(&r11, "fill").unwrap(), "rgb(0, 128, 0)");
        assert_eq!(r11.width, Size::Px(0.0));
        assert_eq!(computed_value(&r11, "rx").unwrap(), "auto");
        assert_eq!(value("st", "stop-color"), "rgb(0, 0, 255)");
        assert_eq!(value("st", "stop-opacity"), "1");
        let s = of("s");
        assert_eq!(
            (s.width.clone(), s.height.clone()),
            (Size::Px(200.0), Size::Px(100.0))
        );
        // An HTML element's attribute is no hint; its properties are
        // there all the same.
        assert_eq!(value("d", "fill"), "rgb(0, 0, 0)");
        let div = style_of("<div>x</div>", &[sheet("div { fill: red }")], "div");
        assert_eq!(computed_value(&div, "fill").unwrap(), "rgb(255, 0, 0)");
    }

    #[test]
    fn the_list_attributes_are_presentational_hints() {
        use crate::computed::ListStyleType;
        let reset = |html: &str, css: &str| {
            style_of(html, &[sheet(css)], "ol")
                .lists()
                .counter_reset
                .iter()
                .map(|r| (r.name.to_string(), r.reversed, r.value))
                .collect::<Vec<_>>()
        };
        let item = |reversed, value| vec![("list-item".to_string(), reversed, value)];
        assert_eq!(reset("<ol start=5></ol>", ""), item(false, Some(4)));
        assert_eq!(reset("<ol start=' -2x'></ol>", ""), item(false, Some(-3)));
        assert_eq!(reset("<ol start=x></ol>", ""), item(false, Some(0)));
        assert_eq!(reset("<ol reversed></ol>", ""), item(true, None));
        assert_eq!(reset("<ol reversed start=3></ol>", ""), item(true, Some(4)));
        assert_eq!(
            reset("<ol start=99999999999></ol>", ""),
            item(false, Some(i32::MAX))
        );
        // An author rule overrides a hint, however unspecific.
        assert_eq!(reset("<ol start=5></ol>", "* { counter-reset: none }"), []);
        let li = style_of("<ol><li value=7 type=a>x</ol>", &[], "li");
        assert_eq!(
            li.lists()
                .counter_set
                .iter()
                .map(|c| (c.name.to_string(), c.value))
                .collect::<Vec<_>>(),
            [("list-item".to_string(), 7)]
        );
        let kind = |html: &str, selector: &str| {
            style_of(html, &[], selector)
                .lists()
                .list_style_type
                .clone()
        };
        let style = |name: &str| ListStyleType::Style(Arc::from(name));
        assert_eq!(
            kind("<ol><li value=7 type=a>x</ol>", "li"),
            style("lower-alpha")
        );
        assert_eq!(kind("<ol type=I></ol>", "ol"), style("upper-roman"));
        assert_eq!(kind("<ol type=i></ol>", "ol"), style("lower-roman"));
        assert_eq!(kind("<ol type=SQUARE></ol>", "ol"), style("decimal"));
        assert_eq!(kind("<ul type=SQUARE></ul>", "ul"), style("square"));
        assert_eq!(kind("<ul type=none></ul>", "ul"), ListStyleType::None);
        assert_eq!(kind("<ul type=a></ul>", "ul"), style("disc"));
        assert_eq!(kind("<p type=a></p>", "p"), style("disc"));
    }

    #[test]
    fn the_text_decoration_properties_parse() {
        // Text Decoration 3 and 4 (M21.7), and the web-platform-tests'
        // parsing cases.
        use crate::computed::{
            DecorationLine, DecorationStyle, DecorationThickness, UnderlinePosition,
        };
        let of = |declarations: &str| {
            p_with("<p>x</p>", &format!("p {{ {declarations} }}"))
                .decoration()
                .clone()
        };
        let line = |u, o, t, b| DecorationLine {
            underline: u,
            overline: o,
            line_through: t,
            blink: b,
        };
        for (value, expected) in [
            ("none", line(false, false, false, false)),
            ("underline", line(true, false, false, false)),
            ("line-through UNDERLINE", line(true, false, true, false)),
            ("blink overline underline", line(true, true, false, true)),
        ] {
            assert_eq!(
                of(&format!("text-decoration-line: {value}")).line,
                expected,
                "{value}"
            );
        }
        for bad in [
            "auto",
            "none underline",
            "underline underline",
            "underline none overline",
            "spelling-error overline",
        ] {
            assert_eq!(
                of(&format!(
                    "text-decoration-line: overline; text-decoration-line: {bad}"
                ))
                .line,
                line(false, true, false, false),
                "{bad}"
            );
        }
        let d = of("text-decoration-style: wavy; text-decoration-color: #00f; \
                    text-decoration-thickness: 2em; text-underline-offset: -10%; \
                    text-underline-position: right under");
        assert_eq!(d.style, DecorationStyle::Wavy);
        assert_eq!(d.color, Some(Color::rgb8(0, 0, 255)));
        assert_eq!(
            d.thickness,
            DecorationThickness::Length(LengthPercentage::Px(32.0))
        );
        assert_eq!(d.underline_offset, Some(LengthPercentage::Percent(-10.0)));
        assert_eq!(d.underline_position, UnderlinePosition::Under);
        let d = of(
            "text-decoration-color: currentcolor; text-decoration-thickness: from-font; \
                    text-underline-offset: auto; text-underline-position: left",
        );
        assert_eq!(d.color, None);
        assert_eq!(d.thickness, DecorationThickness::FromFont);
        assert_eq!(d.underline_offset, None);
        assert_eq!(d.underline_position, UnderlinePosition::Auto);
        for bad in [
            "auto under",
            "left right",
            "under from-font",
            "right under left",
        ] {
            assert_eq!(
                of(&format!(
                    "text-underline-position: under; text-underline-position: {bad}"
                ))
                .underline_position,
                UnderlinePosition::Under,
                "{bad}"
            );
        }
        for bad in ["-10", "-1px", "from font", "otto"] {
            assert_eq!(
                of(&format!("text-decoration-thickness: {bad}")).thickness,
                DecorationThickness::Auto,
                "{bad}"
            );
        }
        // The shorthand: any order, the line's keywords together, each
        // part once; what it leaves out is reset.
        let d = of("text-decoration-style: dotted; text-decoration: overline green from-font");
        assert_eq!(d.line, line(false, true, false, false));
        assert_eq!(d.style, DecorationStyle::Solid);
        assert_eq!(d.color, Some(Color::rgb8(0, 128, 0)));
        assert_eq!(d.thickness, DecorationThickness::FromFont);
        let d = of("text-decoration: double overline underline");
        assert_eq!(
            (d.line, d.style),
            (line(true, true, false, false), DecorationStyle::Double)
        );
        assert_eq!(of("text-decoration: solid").line, DecorationLine::default());
        for bad in [
            "double overline underline dotted",
            "red line-through green",
            "overline blue underline",
            "none underline",
        ] {
            assert_eq!(
                of(&format!(
                    "text-decoration: underline; text-decoration: {bad}"
                ))
                .line,
                line(true, false, false, false),
                "{bad}"
            );
        }
        // The offset and the position are inherited; the rest is not.
        let p = p_with(
            "<div><p>x</p></div>",
            "div { text-decoration: underline red; text-underline-offset: 3px; \
             text-underline-position: under }",
        );
        let d = p.decoration();
        assert_eq!(d.line, DecorationLine::default());
        assert_eq!(d.color, None);
        assert_eq!(d.underline_offset, Some(LengthPercentage::Px(3.0)));
        assert_eq!(d.underline_position, UnderlinePosition::Under);
        assert_eq!(p_with("<p>x</p>", "").decoration, None);
    }

    #[test]
    fn the_ua_sheet_decorates_links_and_edits() {
        use crate::computed::DecorationStyle;
        let html = "<a href=x>a</a><ins>i</ins><u>u</u><del>d</del><s>s</s>\
                    <strike>t</strike><abbr title=x>b</abbr><abbr>c</abbr><a>n</a>";
        let line = |selector: &str| {
            let d = style_of(html, &[], selector).decoration().clone();
            (d.line.underline, d.line.line_through, d.style)
        };
        let (under, through) = ((true, false), (false, true));
        for (selector, expected) in [
            ("a[href]", under),
            ("ins", under),
            ("u", under),
            ("del", through),
            ("s", through),
            ("strike", through),
        ] {
            let (u, t, style) = line(selector);
            assert_eq!(
                ((u, t), style),
                (expected, DecorationStyle::Solid),
                "{selector}"
            );
        }
        assert_eq!(line("abbr[title]"), (true, false, DecorationStyle::Dotted));
        assert!(!line("abbr:not([title])").0);
        assert!(!line("a:not([href])").0);
    }

    /// M35.2: elements made of the same declarations, with parents of the
    /// same style, take the base style the first of them was given,
    /// cousins too, and every element's style is the one a cascade that
    /// keeps nothing gives it, an `attr()` and an inline style among them.
    #[test]
    fn a_style_made_once_is_kept_for_the_elements_made_of_the_same() {
        let html = format!(
            "<ul>{}</ul><table>{}</table><p>x</p><p class=a>y</p>\
             <p data-x=1>z</p><p data-x=2>w</p><p style='color: green'>v</p>             <p style='color: blue'>u</p><div class=a><b>s</b></div><div><b>t</b></div>             <span dir=rtl>r</span><span>l</span>",
            "<li>i</li>".repeat(50),
            "<tr><td>a<td>b<td>c<td>d</tr>".repeat(20)
        );
        let sheets = [sheet(
            "li { margin: 2px } td { padding: 1px } .a { color: red } \
             li:nth-child(2n) { color: blue } p[data-x] { --x: attr(data-x) }",
        )];
        let document = Document::from_node(&html::parse_document(&html).expect("never fails"));
        let afresh = || AFRESH.with(std::cell::Cell::get);
        PLAIN.with(|plain| plain.set(true));
        let reference = cascade_document(&document, &sheets, &CascadeContext::default());
        PLAIN.with(|plain| plain.set(false));
        let before = afresh();
        let kept = cascade_document(&document, &sheets, &CascadeContext::default());
        let made = afresh() - before;
        let elements = document
            .subtree(document.root())
            .filter(|&node| document.is_element(node))
            .count() as u64;
        assert!(made < 26, "{made} of {elements} styles computed");
        for node in document.subtree(document.root()) {
            assert_eq!(reference.get(node), kept.get(node), "{node:?}");
        }
        // The two paragraphs that read their attributes differ.
        let x = |n: usize| {
            let node = document
                .subtree(document.root())
                .filter(|&node| document.attr(node, "data-x").is_some())
                .nth(n)
                .expect("a paragraph");
            kept.get(node).expect("styled").clone()
        };
        assert_ne!(x(0), x(1));
    }
}
