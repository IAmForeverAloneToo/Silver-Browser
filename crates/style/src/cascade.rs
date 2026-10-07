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
//! * Colors: hex (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`) and CSS Color
//!   4's 148 names (M17; eighteen before), not `rgb()`/`hsl()`
//!   functions yet.
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
//!   the properties under a second name ([`canonical_name`]).
//! * Grid (M18): `grid-template-columns`, `grid-template-rows` and
//!   `grid-template-areas`, `grid-auto-columns`, `grid-auto-rows` and
//!   `grid-auto-flow`, the placement properties `grid-row-start`,
//!   `grid-row-end`, `grid-column-start` and `grid-column-end`, and the
//!   shorthands `grid-template`, `grid`, `grid-row`, `grid-column` and
//!   `grid-area`, parsed in [`grid`]; `justify-items` and
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

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use css::Token;
use dom::{Change, Document, NodeId};

use crate::calc::{self, LengthUnit, Want};
use crate::computed::{
    AlignPosition, BOTTOM, BaselinePosition, BorderCollapse, BorderSpacing, BorderStyle, BoxSizing,
    CaptionSide, Clear, Color, ComputedStyle, Content, ContentAlignment, ContentItem, Direction,
    Display, Edges, EmptyCells, FlexBasis, FlexDirection, FlexWrap, Float, LEFT, LengthPercentage,
    LengthPercentageAuto, MarginEdges, MaxSize, Overflow, OverflowAlign, PaddingEdges, Position,
    Quotes, RIGHT, SelfAlignment, Size, TOP, TableLayout, TextAlign, TextTransform, VerticalAlign,
    WhiteSpace,
};
use crate::computed::{Calc, Effects, Fragmentation, GridStyle, JustifyItems};
use crate::invalidation::{Reach, SelectorFeatures, invalidate};
use crate::media::Media;
use crate::page::{PagePosition, PageStyle};
use crate::rule_set::RuleSet;
use crate::selector_matching::{Matcher, PseudoElement, SelectorList, Specificity};

mod effects;
mod fragmentation;
mod grid;
mod page;

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
    /// The answer each `@media` list gave, the UA sheet's then the
    /// author sheets', which decided the rules in force (M20).
    media: Vec<bool>,
}

impl Styles {
    /// The computed style of `node`: an element's, or the document
    /// node's initial values; `None` for a text node, a node not styled
    /// (detached), or a stale id.
    pub fn get(&self, node: NodeId) -> Option<&ComputedStyle> {
        self.computed(node).map(|computed| &computed.style)
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
        let index = node.index();
        if self.entries.len() <= index {
            self.entries.resize(index + 1, None);
        }
        self.entries[index] = Some((node, computed));
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

/// An element's computed style and those of its pseudo-elements.
#[derive(Debug, Clone, PartialEq, Default)]
struct Computed {
    style: ComputedStyle,
    /// `None` when no rule styles any of them, as for most elements.
    pseudo: Option<Box<PseudoStyles>>,
}

/// The styles of an element's pseudo-elements that some rule styles
/// (M16), each cascaded from its own rules with the element's style as
/// the parent it inherits from (Pseudo-Elements 4, tree-abiding
/// pseudo-elements; `::first-letter` and `::first-line` inherit
/// differently when laid out).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PseudoStyles {
    /// `::before`.
    pub before: Option<ComputedStyle>,
    /// `::after`.
    pub after: Option<ComputedStyle>,
    /// `::first-letter`.
    pub first_letter: Option<ComputedStyle>,
    /// `::first-line`.
    pub first_line: Option<ComputedStyle>,
}

impl PseudoStyles {
    /// The style of `pseudo`, if it is one kept and some rule styles it.
    pub fn get(&self, pseudo: PseudoElement) -> Option<&ComputedStyle> {
        match pseudo {
            PseudoElement::Before => self.before.as_ref(),
            PseudoElement::After => self.after.as_ref(),
            PseudoElement::FirstLetter => self.first_letter.as_ref(),
            PseudoElement::FirstLine => self.first_line.as_ref(),
            PseudoElement::Marker
            | PseudoElement::Placeholder
            | PseudoElement::Selection
            | PseudoElement::WebkitPrefixed => None,
        }
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
}

impl Target {
    /// The targets in the order [`Matched`] keeps them.
    const ALL: [Target; 5] = [
        Target::Element,
        Target::Before,
        Target::After,
        Target::FirstLetter,
        Target::FirstLine,
    ];

    /// Whether `content` applies: on `::before` and `::after` only
    /// (CSS 2.2 §12.2).
    fn takes_content(self) -> bool {
        matches!(self, Target::Before | Target::After)
    }

    /// The target a pseudo-element's style is kept as; `None` for the
    /// ones matched and set aside, which nothing draws yet.
    fn of(pseudo: PseudoElement) -> Option<Target> {
        match pseudo {
            PseudoElement::Before => Some(Target::Before),
            PseudoElement::After => Some(Target::After),
            PseudoElement::FirstLetter => Some(Target::FirstLetter),
            PseudoElement::FirstLine => Some(Target::FirstLine),
            PseudoElement::Marker
            | PseudoElement::Placeholder
            | PseudoElement::Selection
            | PseudoElement::WebkitPrefixed => None,
        }
    }

    /// Whether a declaration of the property `name` applies to the
    /// target (M16). `content` applies to `::before` and `::after` only.
    /// `::first-line` takes the properties Pseudo-Elements 4 §2.1.2
    /// lists that this engine has — the font's, colour, background,
    /// `text-transform`, letter and word spacing and `vertical-align` —
    /// and `::first-letter` those of §2.2.4: these, margins, padding and
    /// borders, and `float`, which every browser honours for a drop cap.
    /// The element, `::before` and `::after` take every other property.
    fn applies(self, name: &str) -> bool {
        let line = matches!(
            name,
            "font-size"
                | "font-family"
                | "color"
                | "background-color"
                | "text-transform"
                | "letter-spacing"
                | "word-spacing"
                | "vertical-align"
        );
        match self {
            Target::FirstLine => line,
            Target::FirstLetter => {
                line || name == "float"
                    || (["margin", "padding", "border"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                        // A table's, not the box's border.
                        && !matches!(name, "border-collapse" | "border-spacing"))
            }
            Target::Element => name != "content",
            Target::Before | Target::After => true,
        }
    }
}

/// What the font can say about itself, for the units that need it:
/// `ex` (the x-height) and `ch` (the advance of `0`). `style` has no
/// font of its own (the layering contract: `text` is a sibling, not a
/// dependency), so the caller lends one through this trait; without
/// one, both units fall back to half an em (css-values-4 §6.1.1).
pub trait FontMeasure {
    /// The x-height, in CSS pixels, of the font at `font_size_px`.
    fn x_height(&self, font_size_px: f32) -> f32;
    /// The advance, in CSS pixels, of the digit zero at `font_size_px`.
    fn zero_advance(&self, font_size_px: f32) -> f32;
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
}

/// Resolves the computed style of every element under `document`'s
/// root, cascading the built-in UA stylesheet (lowest precedence) under
/// `author_sheets` (in document order) and each element's inline
/// `style` attribute (highest precedence short of `!important` — see
/// [`tier`]), with the viewport and font units resolved against
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

/// A document's author stylesheets prepared for the cascade (M13.1):
/// each style rule's selectors parsed once, with the conditions and the
/// layer it is in (M20), and what the selectors depend on, the built-in
/// UA sheet's included, which says what a change to the document can
/// restyle. Made when the stylesheets change and kept for every cascade
/// and restyle until they do again.
#[derive(Debug, Clone)]
pub struct Stylesheets {
    sheets: Vec<css::Stylesheet>,
    rules: RuleSet,
    features: SelectorFeatures,
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
        }
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
        self.cascader(document, context).cascade()
    }

    /// Whether the rules in force under `media` differ from those
    /// `styles` were cascaded with: whether any `@media` list, the UA
    /// sheet's or these sheets', answers otherwise (M20). What a caller
    /// asks before reusing styles in a new environment, a resize or a
    /// change of the user's preferences; a few comparisons a list.
    pub fn media_changed(&self, styles: &Styles, media: &Media) -> bool {
        self.answers(media) != styles.media
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
        let resolution = Resolution {
            rem,
            context: &context,
            viewport_used: &viewport_used,
            attributes: &[],
        };
        let bases = LengthBases {
            em: rem,
            resolution: &resolution,
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

    /// The answers of the UA sheet's `@media` lists, then these sheets'.
    fn answers(&self, media: &Media) -> Vec<bool> {
        let mut answers = ua().rules.answers(media);
        answers.extend(self.rules.answers(media));
        answers
    }

    /// A cascader over `document` with the rules in force under
    /// `context`'s medium.
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
        let mut media = ua_answers;
        media.extend(answers);
        Cascader::new(document, ua_rules, author, context, media)
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
        let invalidation = invalidate(document, &self.features, changes);
        let mut cascader = self.cascader(document, context);
        if invalidation.everything || cascader.media != styles.media {
            *styles = cascader.cascade();
            return Restyle::full(styles);
        }
        for removed in &invalidation.removed {
            for node in document.subtree(*removed) {
                styles.forget(node);
            }
        }
        let mut restyle = Restyle::default();
        // By depth, shallowest first: every ancestor's style is final
        // before a node is restyled from its parent's.
        let mut levels: BTreeMap<usize, Vec<NodeId>> = BTreeMap::new();
        let mut pending = invalidation.dirty;
        for &node in pending.keys() {
            levels
                .entry(document.ancestors(node).count())
                .or_default()
                .push(node);
        }
        let first_element = document.document_element();
        while let Some((depth, nodes)) = levels.pop_first() {
            for node in nodes {
                let Some(reach) = pending.remove(&node) else {
                    continue; // listed twice
                };
                let parent_style = document
                    .parent(node)
                    .and_then(|parent| styles.get(parent))
                    .cloned()
                    .unwrap_or_default();
                // The root element resolves `rem` against the initial
                // font size, every other element against the root's.
                let rem = if Some(node) == first_element {
                    ComputedStyle::default().font_size_px
                } else {
                    styles.rem
                };
                // A change to a pseudo-element's style counts as one to
                // the element's: how layout hears of it.
                let computed = cascader.style_of(node, &parent_style, rem);
                restyle.computed += 1;
                let changed = styles.computed(node) != Some(&computed);
                styles.set(node, computed);
                if changed {
                    restyle.changed.push(node);
                }
                if changed || reach == Reach::Subtree {
                    let child_reach = if reach == Reach::Subtree {
                        Reach::Subtree
                    } else {
                        Reach::Element
                    };
                    for child in document.element_children(node) {
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
        let rem = first_element
            .and_then(|root| styles.get(root))
            .map_or(ComputedStyle::default().font_size_px, |s| s.font_size_px);
        if rem != styles.rem {
            *styles = cascader.cascade();
            return Restyle::full(styles);
        }
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
            document,
            styles,
            cascader: RefCell::new(self.cascader(document, context)),
        }
    }
}

impl Recascader<'_> {
    /// `cascade` of `element`'s matched declarations, resolved as the
    /// cascade resolves them: the root element's `rem` is the initial
    /// font size, every other element's the root's.
    fn with_matched<R>(
        &self,
        element: NodeId,
        cascade: impl FnOnce(&Matched, &Resolution<'_>) -> R,
    ) -> R {
        let mut cascader = self.cascader.borrow_mut();
        let cascader = &mut *cascader;
        let matched = declarations_for(
            self.document,
            &mut cascader.matcher,
            element,
            &cascader.ua,
            &cascader.author,
        );
        let rem = if self.document.document_element() == Some(element) {
            ComputedStyle::default().font_size_px
        } else {
            self.styles.rem
        };
        let resolution = Resolution {
            rem,
            context: cascader.context,
            viewport_used: &cascader.viewport_used,
            attributes: self.document.attrs(element).unwrap_or_default(),
        };
        cascade(&matched, &resolution)
    }
}

impl Recascade for Recascader<'_> {
    fn element(&self, element: NodeId, parent: &ComputedStyle) -> ComputedStyle {
        self.with_matched(element, |matched, resolution| {
            element_style(self.document, element, matched, parent, resolution)
        })
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
        self.with_matched(element, |matched, resolution| {
            let applied = matched.of(target);
            (!applied.is_empty()).then(|| compute_style(applied, parent, target, None, resolution))
        })
    }
}

/// An element's style from its `matched` declarations, inheriting from
/// `parent`: its `dir` attribute read, and the root element's `display`
/// blockified.
fn element_style(
    document: &Document,
    element: NodeId,
    matched: &Matched,
    parent: &ComputedStyle,
    resolution: &Resolution<'_>,
) -> ComputedStyle {
    let mut style = compute_style(
        matched.of(Target::Element),
        parent,
        Target::Element,
        document.attr(element, "dir"),
        resolution,
    );
    // The root element's box is a block container or a table, whatever
    // its `display` (CSS2 §9.7, css-display-3 §2.7): what the canvas
    // takes its background from, and every other box is inside.
    if document.document_element() == Some(element) {
        style.display = style.display.blockified();
    }
    style
}

/// What [`Stylesheets::restyle`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Restyle {
    /// The document was cascaded whole: every element counts as
    /// changed, and [`Restyle::changed`] is empty.
    pub full: bool,
    /// The elements whose computed style changed, a newly styled one
    /// among them, in the order they were restyled.
    pub changed: Vec<NodeId>,
    /// How many elements had their style computed.
    pub computed: usize,
}

impl Restyle {
    /// A full cascade, which computed every element of `styles`.
    fn full(styles: &Styles) -> Self {
        Restyle {
            full: true,
            changed: Vec::new(),
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
        let rules = RuleSet::new(std::slice::from_ref(&sheet), &mut features);
        UserAgent {
            sheet,
            rules,
            features,
        }
    })
}

/// The cascade's working state over one document: the matcher, with
/// its caches, and the rules in force.
struct Cascader<'a> {
    document: &'a Document,
    matcher: Matcher<'a>,
    ua: Vec<ParsedRule<'a>>,
    author: Vec<ParsedRule<'a>>,
    context: &'a CascadeContext<'a>,
    /// The answers that left these rules in force, kept with the styles.
    media: Vec<bool>,
    /// Set when a viewport unit is met.
    viewport_used: Cell<bool>,
}

impl<'a> Cascader<'a> {
    fn new(
        document: &'a Document,
        ua: Vec<ParsedRule<'a>>,
        author: Vec<ParsedRule<'a>>,
        context: &'a CascadeContext<'a>,
        media: Vec<bool>,
    ) -> Self {
        Cascader {
            document,
            matcher: Matcher::with_language(document, context.language),
            ua,
            author,
            context,
            media,
            viewport_used: Cell::new(false),
        }
    }

    /// The computed style of `element`, a child of an element (or of the
    /// document node) whose style is `parent_style`, with `rem` the
    /// root element's font size.
    fn style_of(&mut self, element: NodeId, parent_style: &ComputedStyle, rem: f32) -> Computed {
        let matched = declarations_for(
            self.document,
            &mut self.matcher,
            element,
            &self.ua,
            &self.author,
        );
        let resolution = Resolution {
            rem,
            context: self.context,
            viewport_used: &self.viewport_used,
            attributes: self.document.attrs(element).unwrap_or_default(),
        };
        let style = element_style(self.document, element, &matched, parent_style, &resolution);
        // Each pseudo-element some rule styles, from the element's style.
        let pseudo = |target: Target| {
            let applied = matched.of(target);
            (!applied.is_empty()).then(|| compute_style(applied, &style, target, None, &resolution))
        };
        let pseudo = PseudoStyles {
            before: pseudo(Target::Before),
            after: pseudo(Target::After),
            first_letter: pseudo(Target::FirstLetter),
            first_line: pseudo(Target::FirstLine),
        };
        let pseudo = (pseudo != PseudoStyles::default()).then(|| Box::new(pseudo));
        Computed { style, pseudo }
    }

    /// Every element's style, cascaded afresh.
    fn cascade(&mut self) -> Styles {
        let document = self.document;
        self.viewport_used.set(false);
        let mut styles = Styles {
            rem: ComputedStyle::default().font_size_px,
            media: self.media.clone(),
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
        while let Some(parent) = pending.pop() {
            let parent_style = styles.get(parent).cloned().unwrap_or_default();
            let children: Vec<NodeId> = document.element_children(parent).collect();
            for &child in &children {
                let rem = root_font_size.unwrap_or(ComputedStyle::default().font_size_px);
                let computed = self.style_of(child, &parent_style, rem);
                if root_font_size.is_none() {
                    root_font_size = Some(computed.style.font_size_px);
                    styles.rem = computed.style.font_size_px;
                }
                styles.set(child, computed);
            }
            pending.extend(children.into_iter().rev());
        }
        styles.uses_viewport_units = self.viewport_used.get();
        styles
    }
}

/// What one document's relative units resolve against, beyond the
/// element's own font size.
#[derive(Clone, Copy)]
struct Resolution<'a> {
    /// The root element's font size, for `rem`.
    rem: f32,
    context: &'a CascadeContext<'a>,
    /// Set when a viewport unit is met.
    viewport_used: &'a Cell<bool>,
    /// The attributes of the element, or the originating element of the
    /// pseudo-element, the style is for: what `attr()` reads.
    attributes: &'a [(String, String)],
}

/// The bases a length in one element's declarations resolves against:
/// `em` is the element's own font size, except for `font-size` itself,
/// where it is the parent's.
#[derive(Clone, Copy)]
struct LengthBases<'a> {
    em: f32,
    resolution: &'a Resolution<'a>,
}

/// A style rule in force, ready to match: its parsed selectors, its
/// declarations, and its cascade layer's place in the order of normal
/// declarations and of important ones (M20). A rule whose prelude does
/// not parse is dropped before this, per css-syntax-3 (M4.10).
pub(crate) struct ParsedRule<'s> {
    pub(crate) list: &'s SelectorList,
    pub(crate) declarations: &'s [css::Declaration],
    pub(crate) layer: u32,
    pub(crate) important_layer: u32,
}

/// The declarations that apply to an element and to each of its
/// pseudo-elements, unsorted, by [`Target`].
#[derive(Default)]
struct Matched {
    targets: [Vec<Applied>; 5],
}

impl Matched {
    fn of(&self, target: Target) -> &[Applied] {
        &self.targets[target as usize]
    }

    /// The element's own declarations.
    fn element(&mut self) -> &mut Vec<Applied> {
        &mut self.targets[Target::Element as usize]
    }
}

/// A declaration plus everything needed to sort it into cascade order.
struct Applied {
    /// See [`tier`] — encodes origin *and* importance in one order.
    tier: u8,
    /// The place of the rule's cascade layer among those of its origin
    /// and importance (Cascade 5 §6.4); zero for an inline style, which
    /// its tier puts above every layer.
    layer: u32,
    specificity: Specificity,
    /// Strictly increasing in the order this declaration was examined
    /// (UA sheet, then each author sheet, in document order) — the
    /// final tiebreak once tier and specificity are equal.
    order: u32,
    decl: css::Declaration,
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
/// UA `!important` (unused by `assets/ua.css` today, but modeled for
/// correctness — a UA sheet is allowed to use `!important`).
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

/// Every declaration that applies to `element` and to each of its
/// pseudo-elements, tagged with its cascade key — unsorted;
/// [`compute_style`] sorts before applying.
fn declarations_for<'a>(
    document: &Document,
    matcher: &mut Matcher<'a>,
    element: NodeId,
    ua: &[ParsedRule<'a>],
    author: &[ParsedRule<'a>],
) -> Matched {
    let mut order = 0u32;
    let mut out = Matched::default();
    collect_from_sheet(
        matcher,
        element,
        ua,
        TierBase::UserAgent,
        &mut order,
        &mut out,
    );
    collect_from_sheet(
        matcher,
        element,
        author,
        TierBase::AuthorSheet,
        &mut order,
        &mut out,
    );
    // An inline `style` styles the element alone.
    if let Some(style_attr) = document.attr(element, "style")
        && let Ok(decls) = css::parse_style_attribute(style_attr)
    {
        for decl in decls {
            out.element().push(Applied {
                tier: tier(TierBase::AuthorInline, decl.important),
                layer: 0,
                // Inline declarations aren't selector-matched, so there's
                // nothing to compare within this tier by specificity —
                // `order` alone (declaration order in the attribute)
                // breaks ties, matching real engines.
                specificity: Specificity::default(),
                order,
                decl,
            });
            order += 1;
        }
    }
    out
}

/// Matches `sheet`'s rules against `element` and its pseudo-elements,
/// pushing one [`Applied`] per declaration of every rule that matches,
/// for each target it matches.
fn collect_from_sheet<'a>(
    matcher: &mut Matcher<'a>,
    element: NodeId,
    sheet: &[ParsedRule<'a>],
    base: TierBase,
    order: &mut u32,
    out: &mut Matched,
) {
    for rule in sheet {
        // A comma-separated list is cascade-equivalent to separate rules
        // sharing one declaration block — each matching branch
        // contributes independently, so the specificity that matters is
        // the highest among the branches that actually matched here,
        // for the element and for each of its pseudo-elements.
        let list: &'a SelectorList = rule.list;
        let mut best: [Option<Specificity>; 5] = [None; 5];
        for selector in &list.0 {
            let target = match &selector.pseudo_element {
                None => Target::Element,
                // One matched and set aside: nothing draws it yet.
                Some(ending) => match Target::of(ending.element) {
                    Some(target) => target,
                    None => continue,
                },
            };
            let matches = match &selector.pseudo_element {
                None => matcher.matches_element(element, selector),
                Some(ending) => matcher.matches_pseudo_element(element, selector, ending.element),
            };
            if matches {
                let best = &mut best[target as usize];
                *best = (*best).max(Some(selector.specificity));
            }
        }
        if best.iter().all(Option::is_none) {
            continue;
        }
        for (target, specificity) in Target::ALL.into_iter().zip(best) {
            let Some(specificity) = specificity else {
                continue;
            };
            // The same order for each target: within one, it is the
            // declarations' order in the sheets.
            for (offset, decl) in rule.declarations.iter().enumerate() {
                out.targets[target as usize].push(Applied {
                    tier: tier(base, decl.important),
                    layer: if decl.important {
                        rule.important_layer
                    } else {
                        rule.layer
                    },
                    specificity,
                    order: *order + offset as u32,
                    decl: decl.clone(),
                });
            }
        }
        *order += rule.declarations.len() as u32;
    }
}

/// Sorts `applied` into cascade order and folds it into a fresh
/// `ComputedStyle` inherited from `parent`, for `target`.
fn compute_style(
    applied: &[Applied],
    parent: &ComputedStyle,
    target: Target,
    dir_attr: Option<&str>,
    resolution: &Resolution<'_>,
) -> ComputedStyle {
    let mut sorted: Vec<&Applied> = applied.iter().collect();
    sorted.sort_by_key(|a| (a.tier, a.layer, a.specificity, a.order));

    let mut style = ComputedStyle::inherited_from(parent);

    // `dir` (M8.3): a presentational HTML attribute, not a cascaded CSS
    // declaration — applied directly, the same way the inline `style`
    // attribute gets read in `declarations_for`, rather than through
    // `apply_declaration`'s property-name dispatch (there's no CSS
    // `direction` property parser yet to route it through). `auto`
    // (content-sniffed direction) isn't implemented — unrecognized
    // values are ignored, same forward-compat pattern as everywhere
    // else, leaving the inherited direction in place.
    match dir_attr.map(str::to_ascii_lowercase).as_deref() {
        Some("ltr") => style.direction = Direction::Ltr,
        Some("rtl") => style.direction = Direction::Rtl,
        _ => {}
    }

    // `font-size` first: every other length's `em` resolves against
    // *this* element's own font-size, which itself must already be
    // final before those are processed. Its own `em` (and `%`) is the
    // parent's font size.
    let parent_bases = LengthBases {
        em: parent.font_size_px,
        resolution,
    };
    for a in &sorted {
        if a.decl.name != "font-size" {
            continue;
        }
        if let Some(keyword) = css_wide_keyword(&a.decl.value) {
            // `font-size` is inherited, so `unset` means `inherit`.
            style.font_size_px = match keyword {
                CssWide::Initial => ComputedStyle::default().font_size_px,
                CssWide::Inherit | CssWide::Unset => parent.font_size_px,
            };
        } else if let Some(px) = parse_font_size(&a.decl.value, &parent_bases) {
            style.font_size_px = px;
        }
    }

    let bases = LengthBases {
        em: style.font_size_px,
        resolution,
    };
    let mut border_color_set = [false; 4];
    for a in &sorted {
        let name = canonical_name(&a.decl.name);
        if !target.applies(name) {
            continue;
        }
        apply_declaration(
            &mut style,
            name,
            &a.decl.value,
            &bases,
            parent,
            &mut border_color_set,
        );
    }
    // `normal` computes to `none` on `::before` and `::after`.
    if target.takes_content() && style.content == Content::Normal {
        style.content = Content::None;
    }
    for (color, set) in style.border_color.iter_mut().zip(border_color_set) {
        if !set {
            // `border-color`'s initial value is `currentcolor` (css-color-4
            // §4.4) — resolved here now that `color` itself is final.
            *color = style.color;
        }
    }
    // CSS2 §9.7, the relationships between `display`, `position` and
    // `float` (M11): an absolutely positioned box does not float, and
    // a float or an absolutely positioned box takes the block-level
    // `display` of §9.7's table (M12: an inline table becomes a table,
    // a table-internal box a block), so the box tree sees a block
    // container or a table. (The root element's `float` is `none` too;
    // the box tree builder keeps the top-level boxes in flow.)
    if style.position.is_out_of_flow() {
        style.float = Float::None;
    }
    if style.float != Float::None || style.position.is_out_of_flow() {
        style.display = style.display.blockified();
    }
    // The children of a flex or a grid container are its items,
    // blockified (css-display-3 §2.7, css-flexbox-1 §4, M17; css-grid-2
    // §6.1, M18): an element and its `::before` and `::after`, whose
    // parent here is the element. A `::first-letter` or `::first-line`
    // is not one, nor is an element inheriting from a `::first-line`,
    // whose `display` is never a flex or a grid container's.
    if matches!(target, Target::Element | Target::Before | Target::After)
        && parent.display.has_items()
    {
        style.display = style.display.blockified();
    }
    // Grid and effects properties set back to their initial values leave
    // no copy of them behind, so that two styles alike compare equal.
    if style.grid.as_deref() == Some(&GridStyle::default()) {
        style.grid = None;
    }
    if style.effects.as_deref() == Some(&Effects::default()) {
        style.effects = None;
    }
    if style.fragmentation.as_deref() == Some(&Fragmentation::default()) {
        style.fragmentation = None;
    }
    style
}

/// Whether the cascade accepts the declaration `name: value` (M17): a
/// property this engine knows, by its name or a `-webkit-` alias, and a
/// value that property's parser takes, a CSS-wide keyword included.
/// `name` and `value` are a declaration's as `css` parses one: the name
/// lowercased, the value's tokens trimmed and without `!important`. The
/// cascade's own parsers answer, so the answer is what the cascade does
/// with the declaration. A relative length resolves against a 16px font
/// in a viewport of no size, `CascadeContext::default()`'s, which no
/// answer depends on.
pub fn supports(name: &str, value: &[Token]) -> bool {
    let context = CascadeContext::default();
    let viewport_used = Cell::new(false);
    let resolution = Resolution {
        rem: 16.0,
        context: &context,
        viewport_used: &viewport_used,
        attributes: &[],
    };
    let bases = LengthBases {
        em: 16.0,
        resolution: &resolution,
    };
    let parent = ComputedStyle::default();
    apply_declaration(
        &mut ComputedStyle::default(),
        canonical_name(name),
        value,
        &bases,
        &parent,
        &mut [false; 4],
    )
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
        return apply_css_wide(style, name, keyword, parent, border_color_set);
    }
    match name {
        "color" => set(&mut style.color, parse_color(value)),
        "background-color" => set(&mut style.background_color, parse_color(value)),
        "background" => set(&mut style.background_color, parse_background(value)),
        // The first pass of `compute_style` applies it.
        "font-size" => parse_font_size(value, bases).is_some(),
        "font-family" => set(
            &mut style.font_family,
            parse_font_family(value).map(Into::into),
        ),
        "display" => set(&mut style.display, parse_display(value)),
        "text-align" => set(&mut style.text_align, parse_text_align(value)),
        "text-transform" => set(&mut style.text_transform, parse_text_transform(value)),
        "white-space" => set(&mut style.white_space, parse_white_space(value)),
        "letter-spacing" => set(&mut style.letter_spacing, parse_spacing(value, bases)),
        "word-spacing" => set(&mut style.word_spacing, parse_spacing(value, bases)),
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
        "border-color" => {
            let set_colors = set(&mut style.border_color, parse_1_to_4(value, parse_color));
            if set_colors {
                *border_color_set = [true; 4];
            }
            set_colors
        }
        "border-top-color" => set_border_color(style, TOP, value, border_color_set),
        "border-right-color" => set_border_color(style, RIGHT, value, border_color_set),
        "border-bottom-color" => set_border_color(style, BOTTOM, value, border_color_set),
        "border-left-color" => set_border_color(style, LEFT, value, border_color_set),
        "border-style" => set(
            &mut style.border_style,
            parse_1_to_4(value, parse_border_style),
        ),
        "border-top-style" => set_border_style(style, TOP, value),
        "border-right-style" => set_border_style(style, RIGHT, value),
        "border-bottom-style" => set_border_style(style, BOTTOM, value),
        "border-left-style" => set_border_style(style, LEFT, value),
        "border" => apply_border_shorthand(
            style,
            &[TOP, RIGHT, BOTTOM, LEFT],
            value,
            bases,
            border_color_set,
        ),
        "border-top" => apply_border_shorthand(style, &[TOP], value, bases, border_color_set),
        "border-right" => apply_border_shorthand(style, &[RIGHT], value, bases, border_color_set),
        "border-bottom" => apply_border_shorthand(style, &[BOTTOM], value, bases, border_color_set),
        "border-left" => apply_border_shorthand(style, &[LEFT], value, bases, border_color_set),
        "overflow" => set(&mut style.overflow, parse_overflow(value)),
        "float" => set(&mut style.float, parse_float(value)),
        "clear" => set(&mut style.clear, parse_clear(value)),
        "position" => set(&mut style.position, parse_position(value)),
        // Kept as written (M11.3): a percentage is of the containing
        // block's height or width, which only layout knows.
        "top" | "right" | "bottom" | "left" => set(
            &mut style.inset[edge_index(name)],
            parse_length_percentage_or_auto(value, bases),
        ),
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
        "empty-cells" => set(&mut style.empty_cells, parse_empty_cells(value)),
        "vertical-align" => set(
            &mut style.vertical_align,
            parse_vertical_align(value, bases),
        ),
        "content" => set(
            &mut style.content,
            parse_content(value, bases.resolution.attributes),
        ),
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
            let words = idents(&split_whitespace_groups(value));
            let slot = if justify {
                &mut style.justify_content
            } else {
                &mut style.align_content
            };
            set(slot, words.and_then(|w| content_alignment(&w, justify)))
        }
        "align-items" | "align-self" => {
            let items = name == "align-items";
            let words = idents(&split_whitespace_groups(value));
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
            let words = idents(&split_whitespace_groups(value));
            set_grid(
                style,
                words.and_then(|w| grid::justify_items(&w, parent_items)),
                |g, v| g.justify_items = v,
            )
        }
        "justify-self" => {
            let words = idents(&split_whitespace_groups(value));
            set_grid(
                style,
                words.and_then(|w| grid::justify_self(&w, true)),
                |g, v| g.justify_self = v,
            )
        }
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
        "grid-row-start" => set_grid(style, grid::parse_grid_line(value), |g, v| g.row_start = v),
        "grid-row-end" => set_grid(style, grid::parse_grid_line(value), |g, v| g.row_end = v),
        "grid-column-start" => set_grid(style, grid::parse_grid_line(value), |g, v| {
            g.column_start = v
        }),
        "grid-column-end" => set_grid(style, grid::parse_grid_line(value), |g, v| g.column_end = v),
        "grid-row" => set_grid(style, grid::parse_grid_lines(value), |g, (start, end)| {
            g.row_start = start;
            g.row_end = end;
        }),
        "grid-column" => set_grid(style, grid::parse_grid_lines(value), |g, (start, end)| {
            g.column_start = start;
            g.column_end = end;
        }),
        "grid-area" => set_grid(
            style,
            grid::parse_grid_area(value),
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
        "opacity" => set_effects(style, effects::parse_opacity(value, bases), |e, v| {
            e.opacity = v
        }),
        "will-change" => set_effects(style, effects::parse_will_change(value), |e, v| {
            e.will_change = v
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
        // Not in the v1 property set, or not a real property: ignored,
        // never an error.
        _ => false,
    }
}

/// A `font-size` (CSS2 §15.7): a non-negative length, or a percentage
/// of the parent's font size, which is `bases.em` here.
fn parse_font_size(value: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
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

/// [`copy_grid`] for the effects properties.
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
    color_set: &mut [bool; 4],
) -> bool {
    let applied = set(&mut style.border_color[side], parse_color(tokens));
    color_set[side] |= applied;
    applied
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
    let groups = split_whitespace_groups(tokens);
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum CssWide {
    Inherit,
    Initial,
    Unset,
}

fn css_wide_keyword(tokens: &[Token]) -> Option<CssWide> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "inherit" => Some(CssWide::Inherit),
        "initial" => Some(CssWide::Initial),
        "unset" => Some(CssWide::Unset),
        _ => None,
    }
}

/// Whether `name` is an inherited property (css-cascade-5 §7.1 and
/// each property's own definition): what `unset` means for it.
fn is_inherited(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "font-size"
            | "font-family"
            | "text-align"
            | "text-transform"
            | "white-space"
            | "letter-spacing"
            | "word-spacing"
            | "border-collapse"
            | "border-spacing"
            | "caption-side"
            | "empty-cells"
            | "quotes"
            | "orphans"
            | "widows"
            | "print-color-adjust"
    )
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
    match name {
        "color" => style.color = src.color,
        "background-color" | "background" => style.background_color = src.background_color,
        "font-size" => {} // the first pass, see `compute_style`
        "font-family" => style.font_family = src.font_family.clone(),
        "display" => style.display = src.display,
        "text-align" => style.text_align = src.text_align,
        "text-transform" => style.text_transform = src.text_transform,
        "white-space" => style.white_space = src.white_space,
        "letter-spacing" => style.letter_spacing = src.letter_spacing,
        "word-spacing" => style.word_spacing = src.word_spacing,
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
        "overflow" => style.overflow = src.overflow,
        "box-sizing" => style.box_sizing = src.box_sizing,
        "min-width" => style.min_width = src.min_width.clone(),
        "max-width" => style.max_width = src.max_width.clone(),
        "min-height" => style.min_height = src.min_height.clone(),
        "max-height" => style.max_height = src.max_height.clone(),
        "float" => style.float = src.float,
        "clear" => style.clear = src.clear,
        "position" => style.position = src.position,
        "top" | "right" | "bottom" | "left" => {
            let side = edge_index(name);
            style.inset[side] = src.inset[side].clone();
        }
        "z-index" => style.z_index = src.z_index,
        "table-layout" => style.table_layout = src.table_layout,
        "border-collapse" => style.border_collapse = src.border_collapse,
        "border-spacing" => style.border_spacing = src.border_spacing,
        "caption-side" => style.caption_side = src.caption_side,
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
        "opacity" => copy_effects(style, src, |e, s| e.opacity = s.opacity),
        "will-change" => copy_effects(style, src, |e, s| e.will_change = s.will_change),
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
            if from_parent {
                style.border_color = parent.border_color;
            }
            *border_color_set = [from_parent; 4];
        }
        _ => return false, // a property this engine does not know: ignored, as ever
    }
    true
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
    let groups = split_whitespace_groups(tokens);
    let values: Option<Vec<LengthPercentage>> = groups
        .iter()
        .map(|g| parse_non_negative_length_percentage(g, bases))
        .collect();
    expand_1_to_4(&values?)
}

/// Expands the `margin` box shorthand — like [`parse_box_shorthand`],
/// but each value may also be a percentage or `auto`.
fn parse_margin_shorthand(tokens: &[Token], bases: &LengthBases<'_>) -> Option<MarginEdges> {
    let groups = split_whitespace_groups(tokens);
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
    let groups = split_whitespace_groups(tokens);
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
            && let Some(c) = parse_color(&g)
        {
            color = Some(c);
        } else {
            return false; // an unrecognized (or repeated) part: drop the whole shorthand
        }
    }
    for &side in sides {
        style.border_width[side] = width.unwrap_or(3.0);
        style.border_style[side] = border_style.unwrap_or(BorderStyle::None);
        match color {
            Some(c) => {
                style.border_color[side] = c;
                border_color_set[side] = true;
            }
            None => border_color_set[side] = false,
        }
    }
    true
}

/// Splits `tokens` on whitespace into non-empty groups, preserving each
/// group's internal token order — shared by `border`'s and the box
/// shorthands' "one value per space-separated word" parsing. White
/// space inside a function or a block splits nothing, so that
/// `calc(1px + 2px) 3px` is two groups (CSS Syntax 3's component
/// values, M21).
fn split_whitespace_groups(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut depth = Nesting::default();
    for t in tokens {
        if matches!(t, Token::Whitespace) && depth.is_top() {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else {
            depth.step(t);
            current.push(t.clone());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
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
    match unit {
        LengthUnit::Em => value * em,
        LengthUnit::Rem => value * f64::from(r.rem),
        LengthUnit::Ex => {
            value
                * r.context
                    .fonts
                    .map_or(em * 0.5, |f| f64::from(f.x_height(bases.em)))
        }
        LengthUnit::Ch => {
            value
                * r.context
                    .fonts
                    .map_or(em * 0.5, |f| f64::from(f.zero_advance(bases.em)))
        }
        LengthUnit::Vw => viewport(vw),
        LengthUnit::Vh => viewport(vh),
        LengthUnit::Vmin => viewport(vw.min(vh)),
        LengthUnit::Vmax => viewport(vw.max(vh)),
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
    match math(tokens, Want::LengthPercentage, bases, None) {
        Some(calc::Computed::Value(v)) => return Some(LengthPercentage::Px(range.clamp(v))),
        // A percentage below zero is of a basis never below it, so it
        // is fitted to a non-negative range as a length is.
        Some(calc::Computed::Percent(p)) => {
            let p = p as f32;
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

fn parse_display(tokens: &[Token]) -> Option<Display> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
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
        _ => None, // grid/list-item/... land with their milestones
    }
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
    let groups = split_whitespace_groups(tokens);
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

/// `float` (M11.1).
fn parse_float(tokens: &[Token]) -> Option<Float> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" => Some(Float::None),
        "left" => Some(Float::Left),
        "right" => Some(Float::Right),
        _ => None, // `inline-start`/`inline-end` are css-logical's, not parsed
    }
}

/// `clear` (M11.1).
fn parse_clear(tokens: &[Token]) -> Option<Clear> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" => Some(Clear::None),
        "left" => Some(Clear::Left),
        "right" => Some(Clear::Right),
        "both" => Some(Clear::Both),
        _ => None,
    }
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
    let groups = split_whitespace_groups(tokens);
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
    let groups = split_whitespace_groups(tokens);
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
    let words = idents(&split_whitespace_groups(tokens))?;
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
    let words = idents(&split_whitespace_groups(tokens))?;
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
    let words = idents(&split_whitespace_groups(tokens))?;
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
    match split_whitespace_groups(tokens).as_slice() {
        [row] => {
            let row = parse_gap(row, bases)?;
            Some((row.clone(), row))
        }
        [row, column] => Some((parse_gap(row, bases)?, parse_gap(column, bases)?)),
        _ => None,
    }
}

/// The name a property is known by here: a `-webkit-` alias the
/// Compatibility Standard requires (§3.4.1), or a legacy gap name
/// css-align-3 §8.3 requires (M18), as the property itself, every other
/// name as it is.
fn canonical_name(name: &str) -> &str {
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
        "-webkit-transform" => "transform",
        "-webkit-transform-origin" => "transform-origin",
        "-webkit-transform-style" => "transform-style",
        "-webkit-perspective" => "perspective",
        "-webkit-perspective-origin" => "perspective-origin",
        "-webkit-backface-visibility" => "backface-visibility",
        // Color Adjust 1's legacy name.
        "-webkit-print-color-adjust" => "print-color-adjust",
        other => other,
    }
}

/// `overflow` (M9.3): the four keywords, one value for both axes. The
/// two-value form of css-overflow-3 (`overflow: hidden visible`) and
/// `clip` are not accepted yet.
fn parse_overflow(tokens: &[Token]) -> Option<Overflow> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        "hidden" => Some(Overflow::Hidden),
        "scroll" => Some(Overflow::Scroll),
        "auto" => Some(Overflow::Auto),
        _ => None,
    }
}

fn parse_text_align(tokens: &[Token]) -> Option<TextAlign> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "left" => Some(TextAlign::Left),
        "right" => Some(TextAlign::Right),
        "center" => Some(TextAlign::Center),
        _ => None, // `justify`/`start`/`end` land with M21
    }
}

fn parse_text_transform(tokens: &[Token]) -> Option<TextTransform> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None, // full-width/full-size-kana land later
    }
}

fn parse_white_space(tokens: &[Token]) -> Option<WhiteSpace> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None, // `break-spaces` (css-text-4) lands later
    }
}

/// `normal` (resets to `0`) or a length — `letter-spacing`/`word-spacing`
/// share this grammar.
fn parse_spacing(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    if is_normal(tokens) {
        return Some(0.0);
    }
    parse_length_px(tokens, bases)
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

/// A comma-joined font-family list, kept as declared — still not
/// consumed for real font *selection*. M8.1 landed per-*script*
/// fallback (`text::FontCollection`: Latin primary + resolved CJK
/// fallbacks), which is what that milestone's own wording asked for,
/// but it's driven entirely by Unicode script detection on the text
/// itself, never by this property: an author's `font-family: "Noto
/// Sans CJK SC", sans-serif` declaration is parsed and stored here and
/// then simply never read by `text`. Properly honoring it needs more
/// than M8.1 did — `FontCollection` is one primary font per `Engine`,
/// not one per declared family list, so per-element family selection
/// would need its own query-and-cache mechanism alongside the
/// per-script one, plus a fallback-order decision when both a declared
/// family *and* a script-appropriate one could apply to the same run.
/// Not scheduled under a specific milestone yet — flagged here instead
/// of silently staying unread.
fn parse_font_family(tokens: &[Token]) -> Option<String> {
    let mut families = Vec::new();
    let mut current = String::new();
    for t in tokens {
        match t {
            Token::Whitespace => {
                if !current.is_empty() {
                    current.push(' ');
                }
            }
            Token::Comma => {
                let name = current.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                families.push(name);
                current.clear();
            }
            Token::Ident(s) => current.push_str(s),
            Token::String { value, bad: false } => current.push_str(value),
            _ => return None,
        }
    }
    let name = current.trim().to_string();
    if name.is_empty() {
        return None;
    }
    families.push(name);
    Some(families.join(", "))
}

/// Hex colors and a small, deliberately-not-exhaustive named-color set
/// (see the module doc) — filters whitespace, then requires exactly one
/// remaining token (trailing garbage invalidates the value, matching
/// real CSS: a malformed value drops the whole declaration).
/// The `background` shorthand (css-backgrounds-3 §3.10, M17) as far as
/// this engine paints backgrounds: one layer of a `<color>` and the
/// image `none`, each at most once and either left out; the colour it
/// sets, `transparent` when it gives none. Any other component — an
/// image, a position, a size, a second layer — is one this engine cannot
/// honour, so the declaration is dropped rather than half applied.
fn parse_background(tokens: &[Token]) -> Option<Color> {
    let groups = split_whitespace_groups(tokens);
    if groups.is_empty() {
        return None;
    }
    let mut color = None;
    let mut image = false;
    for group in &groups {
        if !image && single_ident(group).is_some_and(|i| i.eq_ignore_ascii_case("none")) {
            image = true;
        } else if color.is_none()
            && let Some(c) = parse_color(group)
        {
            color = Some(c);
        } else {
            return None;
        }
    }
    Some(color.unwrap_or(ComputedStyle::default().background_color))
}

fn parse_color(tokens: &[Token]) -> Option<Color> {
    match single_token(tokens)? {
        Token::Hash { value, .. } => parse_hex_color(value),
        Token::Ident(name) => named_color(name),
        _ => None,
    }
}

/// `content` (CSS 2.2 §12.2, M16): `normal`, `none`, or one or more
/// of a string, `attr(<name>)`, `url()` and the four quote keywords;
/// and after them, css-content-3's `/` and alternative text of strings
/// and `attr()`, accepted and set aside, as nothing reads it. An
/// `attr()` takes the value of the attribute of that name in
/// `attributes`, the name ASCII lowercased as HTML says (§4.16.1), or
/// the empty string when there is none. A `url()` is left out until
/// images (M23). `counter()` and `counters()` make the declaration
/// invalid until counters exist (M21.4), as anything else does.
fn parse_content(tokens: &[Token], attributes: &[(String, String)]) -> Option<Content> {
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
    let mut i = 0;
    while let Some(token) = tokens.get(i) {
        i += 1;
        let item = match token {
            Token::Whitespace => continue,
            Token::Delim('/') if parts > 0 && alternative.is_none() => {
                alternative = Some(0);
                continue;
            }
            Token::String { value, bad: false } => Some(ContentItem::Text(value.clone())),
            Token::Function(name) if name.eq_ignore_ascii_case("attr") => {
                let (name, next) = attr_argument(tokens, i)?;
                i = next;
                let value = attributes
                    .iter()
                    .find(|(attribute, _)| *attribute == name)
                    .map_or("", |(_, value)| value);
                Some(ContentItem::Text(value.to_owned()))
            }
            _ if alternative.is_some() => return None,
            // The tokenizer makes the whole of a `url(…)`, quoted or not,
            // one token.
            Token::Url { bad: false, .. } => None,
            Token::Ident(keyword) => Some(match keyword.to_ascii_lowercase().as_str() {
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

/// The attribute name of an `attr()` whose arguments start at
/// `tokens[i]`, ASCII lowercased, and the index after its `)`: one
/// identifier, with white space around it, and nothing else, the
/// typed and fallback forms of css-values-5 being M21.5's.
fn attr_argument(tokens: &[Token], mut i: usize) -> Option<(String, usize)> {
    let mut name = None;
    while let Some(token) = tokens.get(i) {
        i += 1;
        match token {
            Token::Whitespace => {}
            Token::Ident(ident) if name.is_none() => name = Some(ident.to_ascii_lowercase()),
            Token::CloseParen => return name.map(|name| (name, i)),
            _ => return None,
        }
    }
    None
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

fn single_token(tokens: &[Token]) -> Option<&Token> {
    let mut it = tokens.iter().filter(|t| !matches!(t, Token::Whitespace));
    let first = it.next()?;
    match it.next() {
        None => Some(first),
        Some(_) => None,
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
        3 => Some(Color {
            r: one(0)?,
            g: one(1)?,
            b: one(2)?,
            a: 1.0,
        }),
        4 => Some(Color {
            r: one(0)?,
            g: one(1)?,
            b: one(2)?,
            a: one(3)?,
        }),
        6 => Some(Color {
            r: two(0)?,
            g: two(2)?,
            b: two(4)?,
            a: 1.0,
        }),
        8 => Some(Color {
            r: two(0)?,
            g: two(2)?,
            b: two(4)?,
            a: two(6)?,
        }),
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
    let at = NAMED_COLORS
        .binary_search_by(|(known, _)| known.cmp(&name.as_str()))
        .ok()?;
    let [r, g, b] = NAMED_COLORS[at].1;
    Some(Color::rgb8(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector_matching::{matches_selector, matches_selector_list, parse_selector_list};

    fn sheet(src: &str) -> css::Stylesheet {
        css::parse_stylesheet(src).expect("test stylesheets are valid")
    }

    #[test]
    fn the_background_shorthand_sets_the_colour_and_resets_it() {
        // M17: `background` is its colour here, the rest of the layer at
        // its initial value; a later shorthand without a colour resets
        // an earlier `background-color`, and one the engine cannot honour
        // is dropped whole.
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
            Color::rgb8(0, 128, 0)
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
            ("-webkit-opacity", "0.5"),
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
            ("background", "green, red"),
            ("background", "green red"),
            ("background", "left top green"),
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
            ("color", "rgb(0, 0, 0)"),
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
        };
        let viewport_used = Cell::new(false);
        let resolution = Resolution {
            rem: 16.0,
            context: &context,
            viewport_used: &viewport_used,
            attributes: &[],
        };
        f(&LengthBases {
            em: 16.0,
            resolution: &resolution,
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
        )
        .cascade();
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
        let sheets = [sheet("p { text-align: justify; }")]; // not v1 yet
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.text_align, TextAlign::Left); // initial value, untouched
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
    fn overflow_parses_its_four_keywords_and_is_not_inherited() {
        for (value, expected) in [
            ("visible", Overflow::Visible),
            ("hidden", Overflow::Hidden),
            ("scroll", Overflow::Scroll),
            ("auto", Overflow::Auto),
            ("HIDDEN", Overflow::Hidden),
        ] {
            let sheets = [sheet(&format!("div {{ overflow: {value}; }}"))];
            assert_eq!(
                style_of("<div><p>x</p></div>", &sheets, "div").overflow,
                expected,
                "{value}"
            );
            assert_eq!(
                style_of("<div><p>x</p></div>", &sheets, "p").overflow,
                Overflow::Visible,
                "overflow is not inherited ({value})"
            );
        }
        // An unknown keyword (and the two-value form, not accepted yet)
        // leaves the initial value.
        let sheets = [sheet(
            "div { overflow: clip; } p { overflow: hidden visible; }",
        )];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "div").overflow,
            Overflow::Visible
        );
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").overflow,
            Overflow::Visible
        );
        // `inherit` reads the parent, `initial`/`unset` the initial value.
        let sheets = [sheet("div { overflow: hidden; } p { overflow: inherit; }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").overflow,
            Overflow::Hidden
        );
        let sheets = [sheet(
            "div { overflow: hidden; } p { overflow: hidden; } p { overflow: unset; }",
        )];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").overflow,
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
    fn white_space_parses_all_four_values() {
        for (value, expected) in [
            ("normal", WhiteSpace::Normal),
            ("pre", WhiteSpace::Pre),
            ("pre-wrap", WhiteSpace::PreWrap),
            ("pre-line", WhiteSpace::PreLine),
        ] {
            let sheets = [sheet(&format!("p {{ white-space: {value}; }}"))];
            assert_eq!(
                style_of("<p>x</p>", &sheets, "p").white_space,
                expected,
                "{value}"
            );
        }
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

    /// A font that answers `ex` with 0.4em and `ch` with 0.6em, so a
    /// test can tell the trait's answer from the fallbacks.
    struct StubFont;

    impl FontMeasure for StubFont {
        fn x_height(&self, font_size_px: f32) -> f32 {
            font_size_px * 0.4
        }

        fn zero_advance(&self, font_size_px: f32) -> f32 {
            font_size_px * 0.6
        }
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
        let context = CascadeContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            fonts: Some(&StubFont),
            language: None,
            media: Media::screen(800.0, 600.0),
        };
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
            24.0,
            "a percentage font size is of the parent's (the body's UA 16px)"
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
    fn border_color_defaults_to_currentcolor() {
        let sheets = [sheet("p { color: red; border: 1px solid; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_color, [Color::rgb8(255, 0, 0); 4]);
        // Per side: a side with its own color keeps it, the others follow `color`.
        let sheets = [sheet(
            "p { color: red; border: 1px solid; border-left-color: blue; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_color[LEFT], Color::rgb8(0, 0, 255));
        assert_eq!(s.border_color[TOP], Color::rgb8(255, 0, 0));
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
            Color::BLACK,
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
        assert_eq!(
            content(
                p,
                "'a' attr(title) url(x.png) url('y') OPEN-QUOTE close-quote no-open-quote no-close-quote"
            ),
            items(&[
                text("a"),
                text("T"),
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
        // An image is left out: a box with nothing in it.
        assert_eq!(content(p, "url(x.png)"), items(&[]));
        // Alternative text is accepted and set aside.
        assert_eq!(content(p, "'a' / 'alt' attr(title)"), items(&[text("a")]));
        // Anything else drops the declaration: the earlier one stands.
        for bad in [
            "counter(x)",
            "'a' counters(x, '.')",
            "normal 'a'",
            "none none",
            "attr(x, 'fallback')",
            "attr(x string)",
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
        assert_eq!((s.font_size_px, &*s.font_family), (30.0, "serif"));
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
        assert_eq!(s.white_space, WhiteSpace::PreWrap);
        assert_eq!(s.overflow, Overflow::Visible);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
        assert_eq!(
            (s.border_spacing.horizontal, s.border_spacing.vertical),
            (1.0, 1.0)
        );
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
        // Nothing styles the div's, nor the p's `::first-line`.
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
        assert_eq!((s.font_size_px, &*s.font_family), (30.0, "serif"));
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
            (Float::None, Display::Inline, WhiteSpace::Normal)
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
    fn table_properties_parse_and_inherit_as_the_specification_says() {
        let html = "<div><p>x</p></div>";
        let pair = |css: &str| {
            let sheets = [sheet(css)];
            (style_of(html, &sheets, "div"), style_of(html, &sheets, "p"))
        };
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
        assert_eq!(td.text_align, TextAlign::Left);
        assert_eq!(of("th").text_align, TextAlign::Center);
        assert_eq!(of("th").padding, td.padding);
        assert_eq!(of("caption").text_align, TextAlign::Center);
        // An HTML cell is centred vertically: the row group says
        // `middle`, and `tr`, `td` and `th` inherit it, a row directly in
        // the table included (this parser inserts no `tbody`); a CSS
        // table cell sits on the baseline.
        for selector in [
            "thead", "tbody", "tfoot", "thead tr", "th", "tbody td", "tfoot td",
        ] {
            assert_eq!(
                of(selector).vertical_align,
                VerticalAlign::Middle,
                "{selector}"
            );
        }
        assert_eq!(of("table > tr").vertical_align, VerticalAlign::Middle);
        assert_eq!(of("table > tr > td").vertical_align, VerticalAlign::Middle);
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
        assert_eq!(td.border_color, [Color::BLACK; 4]);
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
        // M10.6: inline-blocks with a border, padding and a background;
        // an input twenty characters wide (no font here: `ch` is half
        // an em of the control's 13.33px) and one line tall, a textarea
        // two lines tall and a scroll container, a select the size of
        // an input with its options hidden, a hidden input nothing, a
        // checkbox a 13px box.
        let html = "<form><input><input type=HIDDEN><input type=checkbox>\
                    <button>Go</button><select><option>a</option></select>\
                    <textarea>t</textarea></form>";
        let control = |selector: &str| style_of(html, &[], selector);
        let input = control("input:first-child");
        assert_eq!(input.display, Display::InlineBlock);
        assert_eq!(input.font_size_px, 13.3333);
        assert_eq!(input.width, Size::Px(20.0 * 0.5 * 13.3333));
        assert_eq!(input.height, Size::Px(1.2 * 13.3333));
        assert_eq!(input.used_border_width(), [1.0; 4]);
        assert_eq!(input.padding[TOP], LengthPercentage::Px(1.0));
        assert_eq!(input.padding[LEFT], LengthPercentage::Px(2.0));
        assert_eq!(input.background_color, Color::rgb8(255, 255, 255));
        assert_eq!(
            control("input[type=HIDDEN]").display,
            Display::None,
            "the UA rule matches the type whatever its case"
        );
        let checkbox = control("input[type=checkbox]");
        assert_eq!(checkbox.width, Size::Px(13.0));
        assert_eq!(checkbox.box_sizing, BoxSizing::BorderBox);
        let button = control("button");
        assert_eq!(button.width, Size::Auto);
        assert_eq!(button.text_align, TextAlign::Center);
        assert_eq!(button.background_color, Color::rgb8(0xef, 0xef, 0xef));
        assert_eq!(control("select").width, input.width);
        assert_eq!(control("option").display, Display::None);
        let textarea = control("textarea");
        assert_eq!(textarea.height, Size::Px(2.4 * 13.3333));
        assert_eq!(textarea.white_space, WhiteSpace::PreWrap);
        assert_eq!(textarea.overflow, Overflow::Auto);
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
            Some(Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            })
        );
        assert_eq!(
            parse_color(&css::tokenize("#ff000080")),
            Some(Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 128.0 / 255.0
            })
        );
        assert_eq!(parse_color(&css::tokenize("#ggg")), None);
        assert_eq!(parse_color(&css::tokenize("#ff")), None); // invalid length
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
        assert_eq!(s.white_space, WhiteSpace::Pre);
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
        // Inside a style rule's block, nothing nests (no CSS nesting).
        let nested = "p { color: blue; @media all { color: red } }";
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
}
