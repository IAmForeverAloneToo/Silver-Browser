//! The cascade: origin sorting, specificity, inheritance, and property
//! parsing — turning a [`dom::Document`] plus its stylesheets into
//! [`Styles`], each element's computed style by its [`dom::NodeId`]
//! (roadmap M5, M13).
//!
//! Scope of this milestone, deliberately:
//!
//! * At-rules (`@media`, `@supports`, ...) are not evaluated — their
//!   rules are skipped entirely for matching purposes (M20 adds real
//!   `@media` evaluation; applying them unconditionally in the meantime
//!   would risk *wrong* results, e.g. `@media print` rules bleeding into
//!   screen rendering, which is worse than not applying them at all).
//! * Colors: hex (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`) and a small,
//!   high-confidence named-color set — not the full CSS Color 148-name
//!   table, and not `rgb()`/`hsl()` functions yet.
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
//!   own v1 scope, plus M10.4's `inline-block`), and the table values
//!   of CSS2 §17.2 (M12). `flex` and `grid` land with their milestones.
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

use crate::computed::{
    BOTTOM, BorderCollapse, BorderSpacing, BorderStyle, BoxSizing, CaptionSide, Clear, Color,
    ComputedStyle, Content, ContentItem, Direction, Display, Edges, EmptyCells, Float, LEFT,
    LengthPercentage, LengthPercentageAuto, MarginEdges, Overflow, PaddingEdges, Position, Quotes,
    RIGHT, TOP, TableLayout, TextAlign, TextTransform, VerticalAlign, WhiteSpace,
};
use crate::invalidation::{Reach, SelectorFeatures, invalidate};
use crate::selector_matching::{
    Matcher, PseudoElement, SelectorList, Specificity, parse_selector_list,
};

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
    /// target. `content` applies to `::before` and `::after` only;
    /// `::first-letter` takes the properties Pseudo-Elements 4 §2.2.4
    /// lists that this engine has — the font's, colour, background,
    /// `text-transform`, letter and word spacing, `vertical-align`,
    /// margins, padding and borders — and `float`, which every browser
    /// honours for a drop cap (M16); every other target takes every
    /// other property.
    fn applies(self, name: &str) -> bool {
        match self {
            Target::FirstLetter => {
                matches!(
                    name,
                    "font-size"
                        | "font-family"
                        | "color"
                        | "background-color"
                        | "text-transform"
                        | "letter-spacing"
                        | "word-spacing"
                        | "vertical-align"
                        | "float"
                ) || (["margin", "padding", "border"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                    // A table's, not the box's border.
                    && !matches!(name, "border-collapse" | "border-spacing"))
            }
            Target::Element | Target::FirstLine => name != "content",
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
    let selectors: Vec<Vec<(usize, SelectorList)>> =
        author_sheets.iter().map(parse_selectors).collect();
    let author = rules_of(author_sheets, &selectors);
    Cascader::new(document, ua().rules.as_slice(), author, context).cascade()
}

/// A document's author stylesheets prepared for the cascade (M13.1):
/// each qualified rule's selectors parsed once, and what the selectors
/// depend on, the built-in UA sheet's included, which says what a
/// change to the document can restyle. Made when the stylesheets
/// change and kept for every cascade and restyle until they do again.
#[derive(Debug, Clone)]
pub struct Stylesheets {
    sheets: Vec<css::Stylesheet>,
    /// For each sheet, each qualified rule whose prelude parses: its
    /// index among the sheet's rules, and its selectors.
    selectors: Vec<Vec<(usize, SelectorList)>>,
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
        let selectors: Vec<Vec<(usize, SelectorList)>> =
            sheets.iter().map(parse_selectors).collect();
        let mut features = ua().features.clone();
        for (_, list) in selectors.iter().flatten() {
            features.add(list);
        }
        for sheet in &sheets {
            features.add_declarations(sheet);
        }
        Stylesheets {
            sheets,
            selectors,
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
        let author = rules_of(&self.sheets, &self.selectors);
        Cascader::new(document, ua().rules.as_slice(), author, context).cascade()
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
    /// give what a full cascade would — the journal overflowed, or the
    /// root's font size, which `rem` reads, changed — the document is
    /// cascaded whole. The result is what a full cascade would give.
    pub fn restyle(
        &self,
        document: &Document,
        context: &CascadeContext<'_>,
        styles: &mut Styles,
        changes: &[Change],
    ) -> Restyle {
        let invalidation = invalidate(document, &self.features, changes);
        let author = rules_of(&self.sheets, &self.selectors);
        let mut cascader = Cascader::new(document, ua().rules.as_slice(), author, context);
        if invalidation.everything {
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
/// content falls (M16): the style of a pseudo-element that inherits
/// from something other than its originating element. A
/// `::first-letter` inherits from the element its text is in, or from
/// the `::first-letter` around it (Pseudo-Elements 4 §2.2.3), which
/// only the box tree finds; `layout` defines no cascade of its own, so
/// it asks through this.
pub trait Recascade {
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
/// cascaded with, and the styles that gave (for `rem`): an element's
/// rules are matched again for each question, which layout asks once per
/// `::first-letter` box.
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
        let author = rules_of(&self.sheets, &self.selectors);
        Recascader {
            document,
            styles,
            cascader: RefCell::new(Cascader::new(
                document,
                ua().rules.as_slice(),
                author,
                context,
            )),
        }
    }
}

impl Recascade for Recascader<'_> {
    fn pseudo_element(
        &self,
        element: NodeId,
        pseudo: PseudoElement,
        parent: &ComputedStyle,
    ) -> Option<ComputedStyle> {
        let target = Target::of(pseudo)?;
        let mut cascader = self.cascader.borrow_mut();
        let cascader = &mut *cascader;
        let matched = declarations_for(
            self.document,
            &mut cascader.matcher,
            element,
            cascader.ua,
            &cascader.author,
        );
        let applied = matched.of(target);
        if applied.is_empty() {
            return None;
        }
        // As the cascade has it: the root element's `rem` is the initial
        // font size, every other element's the root's.
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
        Some(compute_style(applied, parent, target, None, &resolution))
    }
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

/// The built-in user-agent sheet, prepared once per process: its
/// rules, and what their selectors depend on.
struct UserAgent {
    rules: Vec<ParsedRule<'static>>,
    features: SelectorFeatures,
}

fn ua() -> &'static UserAgent {
    static SHEET: OnceLock<(css::Stylesheet, Vec<(usize, SelectorList)>)> = OnceLock::new();
    static UA: OnceLock<UserAgent> = OnceLock::new();
    UA.get_or_init(|| {
        let (sheet, selectors) = SHEET.get_or_init(|| {
            let sheet = css::parse_stylesheet(UA_STYLESHEET_SRC)
                .expect("the built-in UA stylesheet (assets/ua.css) is valid CSS");
            let selectors = parse_selectors(&sheet);
            (sheet, selectors)
        });
        let mut features = SelectorFeatures::default();
        for (_, list) in selectors {
            features.add(list);
        }
        features.add_declarations(sheet);
        UserAgent {
            rules: rules_of(std::slice::from_ref(sheet), std::slice::from_ref(selectors))
                .into_iter()
                .flatten()
                .collect(),
            features,
        }
    })
}

/// The cascade's working state over one document: the matcher, with
/// its caches, and the rules in force.
struct Cascader<'a> {
    document: &'a Document,
    matcher: Matcher<'a>,
    ua: &'a [ParsedRule<'a>],
    author: Vec<Vec<ParsedRule<'a>>>,
    context: &'a CascadeContext<'a>,
    /// Set when a viewport unit is met.
    viewport_used: Cell<bool>,
}

impl<'a> Cascader<'a> {
    fn new(
        document: &'a Document,
        ua: &'a [ParsedRule<'a>],
        author: Vec<Vec<ParsedRule<'a>>>,
        context: &'a CascadeContext<'a>,
    ) -> Self {
        Cascader {
            document,
            matcher: Matcher::with_language(document, context.language),
            ua,
            author,
            context,
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
            self.ua,
            &self.author,
        );
        let resolution = Resolution {
            rem,
            context: self.context,
            viewport_used: &self.viewport_used,
            attributes: self.document.attrs(element).unwrap_or_default(),
        };
        let mut style = compute_style(
            matched.of(Target::Element),
            parent_style,
            Target::Element,
            self.document.attr(element, "dir"),
            &resolution,
        );
        // The root element's box is a block container or a table,
        // whatever its `display` (CSS2 §9.7, css-display-3 §2.7): what
        // the canvas takes its background from, and every other box is
        // inside.
        if self.document.document_element() == Some(element) {
            style.display = style.display.blockified();
        }
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

/// A qualified rule ready to match: its parsed selectors and its
/// declarations. A rule whose prelude does not parse is dropped before
/// this, per css-syntax-3 (M4.10).
struct ParsedRule<'s> {
    list: &'s SelectorList,
    declarations: &'s [css::Declaration],
}

/// Each qualified rule of `sheet` whose prelude parses: its index and
/// its selectors. At-rules are skipped (see the module doc).
fn parse_selectors(sheet: &css::Stylesheet) -> Vec<(usize, SelectorList)> {
    sheet
        .rules
        .iter()
        .enumerate()
        .filter_map(|(i, rule)| match rule {
            css::Rule::Qualified(qr) => Some((i, parse_selector_list(&qr.prelude).ok()?)),
            css::Rule::At(_) => None,
        })
        .collect()
}

/// The rules of `sheets` ready to match, from their `selectors` as
/// [`parse_selectors`] gave them.
fn rules_of<'s>(
    sheets: &'s [css::Stylesheet],
    selectors: &'s [Vec<(usize, SelectorList)>],
) -> Vec<Vec<ParsedRule<'s>>> {
    sheets
        .iter()
        .zip(selectors)
        .map(|(sheet, lists)| {
            lists
                .iter()
                .filter_map(|(i, list)| match &sheet.rules[*i] {
                    css::Rule::Qualified(qr) => Some(ParsedRule {
                        list,
                        declarations: &qr.declarations,
                    }),
                    css::Rule::At(_) => None,
                })
                .collect()
        })
        .collect()
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
/// declarations can be sorted by `(tier, specificity, order)` and
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
    author_sheets: &[Vec<ParsedRule<'a>>],
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
    for sheet in author_sheets {
        collect_from_sheet(
            matcher,
            element,
            sheet,
            TierBase::AuthorSheet,
            &mut order,
            &mut out,
        );
    }
    // An inline `style` styles the element alone.
    if let Some(style_attr) = document.attr(element, "style")
        && let Ok(decls) = css::parse_style_attribute(style_attr)
    {
        for decl in decls {
            out.element().push(Applied {
                tier: tier(TierBase::AuthorInline, decl.important),
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
    sorted.sort_by_key(|a| (a.tier, a.specificity, a.order));

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
        } else if let Some(Token::Percentage { value }) = single_token(&a.decl.value) {
            let px = *value as f32 / 100.0 * parent.font_size_px;
            if px.is_finite() && px >= 0.0 {
                style.font_size_px = px.min(MAX_LENGTH_PX);
            }
        } else if let Some(px) = parse_non_negative_length_px(&a.decl.value, &parent_bases) {
            style.font_size_px = px;
        }
    }

    let bases = LengthBases {
        em: style.font_size_px,
        resolution,
    };
    let mut border_color_set = [false; 4];
    for a in &sorted {
        if !target.applies(&a.decl.name) {
            continue;
        }
        apply_declaration(&mut style, &a.decl, &bases, parent, &mut border_color_set);
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
    style
}

/// Applies one already-cascade-ordered declaration to `style`, ignoring
/// (never erroring on) any property name or value this engine doesn't
/// understand yet — the M3 forward-compatibility pattern: a browser
/// encounters unsupported properties on every real page, constantly.
fn apply_declaration(
    style: &mut ComputedStyle,
    decl: &css::Declaration,
    bases: &LengthBases<'_>,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) {
    if let Some(keyword) = css_wide_keyword(&decl.value) {
        apply_css_wide(style, &decl.name, keyword, parent, border_color_set);
        return;
    }
    match decl.name.as_str() {
        "color" => {
            if let Some(c) = parse_color(&decl.value) {
                style.color = c;
            }
        }
        "background-color" => {
            if let Some(c) = parse_color(&decl.value) {
                style.background_color = c;
            }
        }
        "font-size" => {} // first pass, above
        "font-family" => {
            if let Some(f) = parse_font_family(&decl.value) {
                style.font_family = f;
            }
        }
        "display" => {
            if let Some(d) = parse_display(&decl.value) {
                style.display = d;
            }
        }
        "text-align" => {
            if let Some(a) = parse_text_align(&decl.value) {
                style.text_align = a;
            }
        }
        "text-transform" => {
            if let Some(t) = parse_text_transform(&decl.value) {
                style.text_transform = t;
            }
        }
        "white-space" => {
            if let Some(w) = parse_white_space(&decl.value) {
                style.white_space = w;
            }
        }
        "letter-spacing" => {
            if let Some(v) = parse_spacing(&decl.value, bases) {
                style.letter_spacing = v;
            }
        }
        "word-spacing" => {
            if let Some(v) = parse_spacing(&decl.value, bases) {
                style.word_spacing = v;
            }
        }
        "width" => {
            if let Some(w) = parse_length_percentage_or_auto(&decl.value, bases) {
                style.width = w;
            }
        }
        "height" => {
            // A percentage is kept as one (M10.1); layout resolves it
            // against the containing block's height when that is
            // definite, and treats it as `auto` otherwise (CSS2 §10.5).
            if let Some(h) = parse_length_percentage_or_auto(&decl.value, bases) {
                style.height = h;
            }
        }
        "margin" => {
            if let Some(e) = parse_margin_shorthand(&decl.value, bases) {
                style.margin = e;
            }
        }
        "margin-top" => set_margin_edge(&mut style.margin, TOP, &decl.value, bases),
        "margin-right" => set_margin_edge(&mut style.margin, RIGHT, &decl.value, bases),
        "margin-bottom" => set_margin_edge(&mut style.margin, BOTTOM, &decl.value, bases),
        "margin-left" => set_margin_edge(&mut style.margin, LEFT, &decl.value, bases),
        "padding" => {
            if let Some(e) = parse_padding_shorthand(&decl.value, bases) {
                style.padding = e;
            }
        }
        "padding-top" => set_padding_edge(&mut style.padding, TOP, &decl.value, bases),
        "padding-right" => set_padding_edge(&mut style.padding, RIGHT, &decl.value, bases),
        "padding-bottom" => set_padding_edge(&mut style.padding, BOTTOM, &decl.value, bases),
        "padding-left" => set_padding_edge(&mut style.padding, LEFT, &decl.value, bases),
        "border-width" => {
            if let Some(e) = parse_border_width_shorthand(&decl.value, bases) {
                style.border_width = e;
            }
        }
        "border-top-width" => set_border_width(style, TOP, &decl.value, bases),
        "border-right-width" => set_border_width(style, RIGHT, &decl.value, bases),
        "border-bottom-width" => set_border_width(style, BOTTOM, &decl.value, bases),
        "border-left-width" => set_border_width(style, LEFT, &decl.value, bases),
        "border-color" => {
            if let Some(colors) = parse_1_to_4(&decl.value, parse_color) {
                style.border_color = colors;
                *border_color_set = [true; 4];
            }
        }
        "border-top-color" => set_border_color(style, TOP, &decl.value, border_color_set),
        "border-right-color" => set_border_color(style, RIGHT, &decl.value, border_color_set),
        "border-bottom-color" => set_border_color(style, BOTTOM, &decl.value, border_color_set),
        "border-left-color" => set_border_color(style, LEFT, &decl.value, border_color_set),
        "border-style" => {
            if let Some(styles) = parse_1_to_4(&decl.value, parse_border_style) {
                style.border_style = styles;
            }
        }
        "border-top-style" => set_border_style(style, TOP, &decl.value),
        "border-right-style" => set_border_style(style, RIGHT, &decl.value),
        "border-bottom-style" => set_border_style(style, BOTTOM, &decl.value),
        "border-left-style" => set_border_style(style, LEFT, &decl.value),
        "border" => apply_border_shorthand(
            style,
            &[TOP, RIGHT, BOTTOM, LEFT],
            &decl.value,
            bases,
            border_color_set,
        ),
        "border-top" => apply_border_shorthand(style, &[TOP], &decl.value, bases, border_color_set),
        "border-right" => {
            apply_border_shorthand(style, &[RIGHT], &decl.value, bases, border_color_set)
        }
        "border-bottom" => {
            apply_border_shorthand(style, &[BOTTOM], &decl.value, bases, border_color_set)
        }
        "border-left" => {
            apply_border_shorthand(style, &[LEFT], &decl.value, bases, border_color_set)
        }
        "overflow" => {
            if let Some(o) = parse_overflow(&decl.value) {
                style.overflow = o;
            }
        }
        "float" => {
            if let Some(f) = parse_float(&decl.value) {
                style.float = f;
            }
        }
        "clear" => {
            if let Some(c) = parse_clear(&decl.value) {
                style.clear = c;
            }
        }
        "position" => {
            if let Some(p) = parse_position(&decl.value) {
                style.position = p;
            }
        }
        "top" | "right" | "bottom" | "left" => {
            // Kept as written (M11.3): a percentage is of the containing
            // block's height or width, which only layout knows.
            if let Some(v) = parse_length_percentage_or_auto(&decl.value, bases) {
                style.inset[edge_index(&decl.name)] = v;
            }
        }
        "z-index" => {
            if let Some(z) = parse_z_index(&decl.value) {
                style.z_index = z;
            }
        }
        "box-sizing" => {
            if let Some(b) = parse_box_sizing(&decl.value) {
                style.box_sizing = b;
            }
        }
        "min-width" => {
            if let Some(v) = parse_non_negative_length_percentage(&decl.value, bases) {
                style.min_width = v;
            }
        }
        "min-height" => {
            if let Some(v) = parse_non_negative_length_percentage(&decl.value, bases) {
                style.min_height = v;
            }
        }
        "max-width" => {
            if let Some(v) = parse_max_size(&decl.value, bases) {
                style.max_width = v;
            }
        }
        "max-height" => {
            if let Some(v) = parse_max_size(&decl.value, bases) {
                style.max_height = v;
            }
        }
        "table-layout" => {
            if let Some(t) = parse_table_layout(&decl.value) {
                style.table_layout = t;
            }
        }
        "border-collapse" => {
            if let Some(b) = parse_border_collapse(&decl.value) {
                style.border_collapse = b;
            }
        }
        "border-spacing" => {
            if let Some(s) = parse_border_spacing(&decl.value, bases) {
                style.border_spacing = s;
            }
        }
        "caption-side" => {
            if let Some(c) = parse_caption_side(&decl.value) {
                style.caption_side = c;
            }
        }
        "empty-cells" => {
            if let Some(e) = parse_empty_cells(&decl.value) {
                style.empty_cells = e;
            }
        }
        "vertical-align" => {
            if let Some(v) = parse_vertical_align(&decl.value, bases) {
                style.vertical_align = v;
            }
        }
        "content" => {
            if let Some(c) = parse_content(&decl.value, bases.resolution.attributes) {
                style.content = c;
            }
        }
        "quotes" => {
            if let Some(q) = parse_quotes(&decl.value) {
                style.quotes = q;
            }
        }
        _ => {} // not in the v1 property set (or not a real property) — ignored, never an error
    }
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
) {
    if let Some(v) = parse_border_width(tokens, bases) {
        style.border_width[side] = v;
    }
}

/// Writes a `border-*-color` longhand.
fn set_border_color(style: &mut ComputedStyle, side: usize, tokens: &[Token], set: &mut [bool; 4]) {
    if let Some(c) = parse_color(tokens) {
        style.border_color[side] = c;
        set[side] = true;
    }
}

/// Writes a `border-*-style` longhand.
fn set_border_style(style: &mut ComputedStyle, side: usize, tokens: &[Token]) {
    if let Some(s) = parse_border_style(tokens) {
        style.border_style[side] = s;
    }
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
) {
    if let Some(v) = parse_non_negative_length_percentage(tokens, bases) {
        edges[which] = v;
    }
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
    )
}

/// Applies `name: <keyword>`: the parent's computed value for
/// `inherit`, the initial value for `initial`, whichever the property's
/// inheritance says for `unset`. `font-size` is handled in the first
/// pass of [`compute_style`] and ignored here.
fn apply_css_wide(
    style: &mut ComputedStyle,
    name: &str,
    keyword: CssWide,
    parent: &ComputedStyle,
    border_color_set: &mut [bool; 4],
) {
    let initial = ComputedStyle::default();
    let from_parent = match keyword {
        CssWide::Inherit => true,
        CssWide::Initial => false,
        CssWide::Unset => is_inherited(name),
    };
    let src = if from_parent { parent } else { &initial };
    match name {
        "color" => style.color = src.color,
        "background-color" => style.background_color = src.background_color,
        "font-size" => {} // the first pass, see `compute_style`
        "font-family" => style.font_family = src.font_family.clone(),
        "display" => style.display = src.display,
        "text-align" => style.text_align = src.text_align,
        "text-transform" => style.text_transform = src.text_transform,
        "white-space" => style.white_space = src.white_space,
        "letter-spacing" => style.letter_spacing = src.letter_spacing,
        "word-spacing" => style.word_spacing = src.word_spacing,
        "width" => style.width = src.width,
        "height" => style.height = src.height,
        "margin" => style.margin = src.margin,
        "margin-top" => style.margin[TOP] = src.margin[TOP],
        "margin-right" => style.margin[RIGHT] = src.margin[RIGHT],
        "margin-bottom" => style.margin[BOTTOM] = src.margin[BOTTOM],
        "margin-left" => style.margin[LEFT] = src.margin[LEFT],
        "padding" => style.padding = src.padding,
        "padding-top" => style.padding[TOP] = src.padding[TOP],
        "padding-right" => style.padding[RIGHT] = src.padding[RIGHT],
        "padding-bottom" => style.padding[BOTTOM] = src.padding[BOTTOM],
        "padding-left" => style.padding[LEFT] = src.padding[LEFT],
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
        "min-width" => style.min_width = src.min_width,
        "max-width" => style.max_width = src.max_width,
        "min-height" => style.min_height = src.min_height,
        "max-height" => style.max_height = src.max_height,
        "float" => style.float = src.float,
        "clear" => style.clear = src.clear,
        "position" => style.position = src.position,
        "top" | "right" | "bottom" | "left" => {
            let side = edge_index(name);
            style.inset[side] = src.inset[side];
        }
        "z-index" => style.z_index = src.z_index,
        "table-layout" => style.table_layout = src.table_layout,
        "border-collapse" => style.border_collapse = src.border_collapse,
        "border-spacing" => style.border_spacing = src.border_spacing,
        "caption-side" => style.caption_side = src.caption_side,
        "empty-cells" => style.empty_cells = src.empty_cells,
        "vertical-align" => style.vertical_align = src.vertical_align,
        "content" => style.content = src.content.clone(),
        "quotes" => style.quotes = src.quotes.clone(),
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
        _ => {} // a property this engine does not know: ignored, as ever
    }
}

/// Like [`set_edge`], for a `margin-*` longhand (which additionally
/// accepts `auto`).
fn set_margin_edge(
    edges: &mut MarginEdges,
    which: usize,
    tokens: &[Token],
    bases: &LengthBases<'_>,
) {
    if let Some(v) = parse_length_percentage_or_auto(tokens, bases) {
        edges[which] = v;
    }
}

/// Expands a 1–4 value list into `[top, right, bottom, left]` per CSS2
/// §8.3's box-shorthand expansion rule (shared by `margin`, `padding`,
/// and `border-width` — only the per-value parser differs).
fn expand_1_to_4<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    match values {
        [a] => Some([*a, *a, *a, *a]),
        [a, b] => Some([*a, *b, *a, *b]),
        [a, b, c] => Some([*a, *b, *c, *b]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
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
) {
    let mut width = None;
    let mut border_style = None;
    let mut color = None;
    let groups = split_whitespace_groups(tokens);
    if groups.is_empty() {
        return; // `border: ;` has no component: an invalid declaration, ignored
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
            return; // an unrecognized (or repeated) part: drop the whole shorthand
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
}

/// Splits `tokens` on whitespace into non-empty groups, preserving each
/// group's internal token order — shared by `border`'s and the box
/// shorthands' "one value per space-separated word" parsing.
fn split_whitespace_groups(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for t in tokens {
        if matches!(t, Token::Whitespace) {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else {
            current.push(t.clone());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
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
    let px = match single_token(tokens)? {
        Token::Dimension { value, unit } => {
            let value = *value as f32;
            let r = bases.resolution;
            let (vw, vh) = (r.context.viewport_width, r.context.viewport_height);
            let viewport = |fraction: f32| {
                r.viewport_used.set(true);
                fraction * value / 100.0
            };
            match unit.to_ascii_lowercase().as_str() {
                "px" => value,
                "in" => value * 96.0,
                "cm" => value * 96.0 / 2.54,
                "mm" => value * 96.0 / 25.4,
                "q" => value * 96.0 / 25.4 / 4.0,
                "pt" => value * 96.0 / 72.0,
                "pc" => value * 16.0,
                "em" => value * bases.em,
                "rem" => value * r.rem,
                "ex" => {
                    value
                        * r.context
                            .fonts
                            .map_or(bases.em * 0.5, |f| f.x_height(bases.em))
                }
                "ch" => {
                    value
                        * r.context
                            .fonts
                            .map_or(bases.em * 0.5, |f| f.zero_advance(bases.em))
                }
                "vw" => viewport(vw),
                "vh" => viewport(vh),
                "vmin" => viewport(vw.min(vh)),
                "vmax" => viewport(vw.max(vh)),
                _ => return None,
            }
        }
        Token::Number { value, .. } if *value == 0.0 => 0.0,
        _ => return None,
    };
    if !px.is_finite() {
        return None;
    }
    Some(px.clamp(-MAX_LENGTH_PX, MAX_LENGTH_PX))
}

/// [`parse_length_px`] for the properties that take no negative value
/// (`font-size`, `padding`, `border-width`): a negative one is an
/// invalid declaration, ignored.
fn parse_non_negative_length_px(tokens: &[Token], bases: &LengthBases<'_>) -> Option<f32> {
    parse_length_px(tokens, bases).filter(|v| *v >= 0.0)
}

/// A non-negative length or percentage, kept as written — `padding`.
fn parse_non_negative_length_percentage(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<LengthPercentage> {
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        let p = *value as f32;
        return (p.is_finite() && p >= 0.0).then_some(LengthPercentage::Percent(p));
    }
    parse_non_negative_length_px(tokens, bases).map(LengthPercentage::Px)
}

/// `auto`, a length, or a percentage — `width`, `height` and the
/// margins (M10.1). A percentage is kept as written for layout to
/// resolve against the containing block.
fn parse_length_percentage_or_auto(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<LengthPercentageAuto> {
    if is_auto(tokens) {
        return Some(LengthPercentageAuto::Auto);
    }
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        let p = *value as f32;
        return p.is_finite().then_some(LengthPercentageAuto::Percent(p));
    }
    parse_length_px(tokens, bases).map(LengthPercentageAuto::Px)
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
        _ => None, // flex/grid/list-item/... land with their milestones
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
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        let p = *value as f32;
        return p.is_finite().then_some(VerticalAlign::Percent(p));
    }
    parse_length_px(tokens, bases).map(VerticalAlign::Length)
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
fn parse_z_index(tokens: &[Token]) -> Option<Option<i32>> {
    if is_auto(tokens) {
        return Some(None);
    }
    match single_token(tokens)? {
        Token::Number {
            value,
            is_integer: true,
        } if *value >= i32::MIN as f64 && *value <= i32::MAX as f64 => Some(Some(*value as i32)),
        _ => None,
    }
}

/// `box-sizing` (M10.2).
fn parse_box_sizing(tokens: &[Token]) -> Option<BoxSizing> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

/// `max-width`/`max-height` (M10.3): `none`, or a non-negative length
/// or percentage. The outer `Option` is "parsed at all"; the inner is
/// the value, `None` for `none`.
fn parse_max_size(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Option<LengthPercentage>> {
    if single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    parse_non_negative_length_percentage(tokens, bases).map(Some)
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

/// A small, high-confidence set of named colors (CSS1/CSS2's basic
/// keywords, plus `orange` and `transparent`) — not the full CSS Color
/// 148-name table (deliberately: getting an obscure one wrong silently
/// would be worse than not supporting it yet).
fn named_color(name: &str) -> Option<Color> {
    Some(match name.to_ascii_lowercase().as_str() {
        "black" => Color::rgb8(0, 0, 0),
        "silver" => Color::rgb8(192, 192, 192),
        "gray" | "grey" => Color::rgb8(128, 128, 128),
        "white" => Color::rgb8(255, 255, 255),
        "maroon" => Color::rgb8(128, 0, 0),
        "red" => Color::rgb8(255, 0, 0),
        "purple" => Color::rgb8(128, 0, 128),
        "fuchsia" | "magenta" => Color::rgb8(255, 0, 255),
        "green" => Color::rgb8(0, 128, 0),
        "lime" => Color::rgb8(0, 255, 0),
        "olive" => Color::rgb8(128, 128, 0),
        "yellow" => Color::rgb8(255, 255, 0),
        "navy" => Color::rgb8(0, 0, 128),
        "blue" => Color::rgb8(0, 0, 255),
        "teal" => Color::rgb8(0, 128, 128),
        "aqua" | "cyan" => Color::rgb8(0, 255, 255),
        "orange" => Color::rgb8(255, 165, 0),
        "transparent" => Color::TRANSPARENT,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector_matching::{matches_selector, matches_selector_list};

    fn sheet(src: &str) -> css::Stylesheet {
        css::parse_stylesheet(src).expect("test stylesheets are valid")
    }

    /// Runs `f` with the length bases of a 16px element whose root is
    /// 16px, in an 800 by 600 viewport, with no font.
    fn with_bases<R>(f: impl FnOnce(&LengthBases<'_>) -> R) -> R {
        let context = CascadeContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            fonts: None,
            language: None,
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
        let ua_selectors: Vec<_> = ua.iter().map(parse_selectors).collect();
        let ua_rules: Vec<ParsedRule<'_>> =
            rules_of(&ua, &ua_selectors).into_iter().flatten().collect();
        let author_selectors: Vec<_> = author.iter().map(parse_selectors).collect();
        let context = CascadeContext::default();
        let styles = Cascader::new(
            &document,
            &ua_rules,
            rules_of(&author, &author_selectors),
            &context,
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
            [LengthPercentage::Px(0.0); 4],
            "negative padding is ignored"
        );
        assert_eq!(
            s.border_width, [3.0; 4],
            "negative border-width is ignored: medium stays"
        );
        assert_eq!(s.width, LengthPercentageAuto::Px(MAX_LENGTH_PX));
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
        assert_eq!(p.inset, [LengthPercentageAuto::Auto; 4]);
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
        assert_eq!(s.min_width, LengthPercentage::Px(10.0));
        assert_eq!(s.max_width, Some(LengthPercentage::Percent(50.0)));
        assert_eq!(s.min_height, LengthPercentage::Px(32.0));
        assert_eq!(s.max_height, None);
        // Negative sizes and unknown keywords are ignored; `max-width:
        // none` undoes an earlier value; the properties are not
        // inherited and take the css-wide keywords.
        let sheets = [sheet(
            "div { max-width: 100px; box-sizing: border-box; }
             p { min-width: -1px; max-height: -5%; box-sizing: padding-box; max-width: 40px; }
             p { max-width: none; }",
        )];
        let s = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(s.min_width, LengthPercentage::Px(0.0));
        assert_eq!(s.max_height, None);
        assert_eq!(s.box_sizing, BoxSizing::ContentBox);
        assert_eq!(s.max_width, None);
        let sheets = [sheet("div { max-width: 100px; } p { max-width: inherit; }")];
        assert_eq!(
            style_of("<div><p>x</p></div>", &sheets, "p").max_width,
            Some(LengthPercentage::Px(100.0))
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
        assert_eq!(auto.width, LengthPercentageAuto::Auto); // initial value

        let sheets = [sheet("p { width: 200px; }")];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "p").width,
            LengthPercentageAuto::Px(200.0)
        );

        let sheets = [sheet("p { width: 50%; }")];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "p").width,
            LengthPercentageAuto::Percent(50.0)
        );
    }

    #[test]
    fn height_margins_and_padding_keep_percentages_for_layout() {
        let sheets = [sheet("p { height: 300px; }")];
        assert_eq!(
            style_of("<p>x</p>", &sheets, "p").height,
            LengthPercentageAuto::Px(300.0)
        );
        // A percentage is kept as written (M10.1): only layout knows the
        // containing block it is a percentage of.
        let sheets = [sheet(
            "p { height: 50%; margin: 5% auto; padding: 10% 2px; }",
        )];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.height, LengthPercentageAuto::Percent(50.0));
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
        assert_eq!(s.padding, [LengthPercentage::Px(0.0); 4]);
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
        assert_eq!(s.width, Px(96.0), "an inch is 96px");
        assert_eq!(s.letter_spacing, 16.0, "12pt is 16px");
        for (value, px) in [
            ("2.54cm", 96.0),
            ("25.4mm", 96.0),
            ("40q", 96.0 / 2.54),
            ("1pc", 16.0),
        ] {
            let sheets = [sheet(&format!("p {{ width: {value} }}"))];
            let w = style_of_in("<p>x</p>", &sheets, "p", &context).width;
            let LengthPercentageAuto::Px(got) = w else {
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
        assert_eq!(p.width, LengthPercentageAuto::Px(0.0));
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
        assert_eq!(s.width, LengthPercentageAuto::Auto);
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
            "p { transform: rotate(5deg); color: red; margin: banana; }",
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
        assert_eq!((s.font_size_px, s.font_family.as_str()), (30.0, "serif"));
        assert_eq!(s.text_transform, TextTransform::Uppercase);
        assert_eq!((s.letter_spacing, s.word_spacing), (2.0, 3.0));
        assert_eq!(s.vertical_align, VerticalAlign::Top);
        assert_eq!(s.margin[TOP], LengthPercentageAuto::Px(2.0));
        assert_eq!(s.padding[LEFT], LengthPercentage::Px(3.0));
        assert_eq!(s.border_style, [BorderStyle::Solid; 4]);
        assert_eq!(s.float, Float::Left);
        // A float's `display` is blockified from the initial `inline`.
        assert_eq!(s.display, Display::Block);
        assert_eq!(s.width, LengthPercentageAuto::Auto);
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
            let parent = styles.get(element).expect("styled");
            let cascaded = styles.pseudo(element, PseudoElement::FirstLetter).unwrap();
            let recascaded = recascade
                .pseudo_element(element, PseudoElement::FirstLetter, parent)
                .unwrap();
            (cascaded.font_size_px, recascaded.font_size_px)
        };
        assert_eq!(size("html"), (32.0, 32.0));
        assert_eq!(size("p"), (40.0, 40.0));
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
        ] {
            let sheets = [sheet(&format!("span {{ display: {value}; }}"))];
            assert_eq!(style_of(html, &sheets, "span").display, expected, "{value}");
        }
        // An unknown keyword leaves the UA value in place.
        let sheets = [sheet("div { display: flex; }")];
        assert_eq!(style_of(html, &sheets, "div").display, Display::Block);
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
        assert_eq!(td.padding, [LengthPercentage::Px(1.0); 4]);
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
        assert_eq!(of(".cell").padding, [LengthPercentage::Px(0.0); 4]);
        // Row groups and rows take the table's border colour; a cell
        // keeps `currentcolor`.
        let red = Color::rgb8(255, 0, 0);
        assert_eq!(of("tbody").border_color, [red; 4]);
        assert_eq!(of("tbody tr").border_color, [red; 4]);
        assert_eq!(td.border_color, [Color::BLACK; 4]);
        // The caption and the column boxes have no padding or border of
        // their own from the sheet.
        assert_eq!(of("caption").padding, [LengthPercentage::Px(0.0); 4]);
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
        assert_eq!(input.width, LengthPercentageAuto::Px(20.0 * 0.5 * 13.3333));
        assert_eq!(input.height, LengthPercentageAuto::Px(1.2 * 13.3333));
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
        assert_eq!(checkbox.width, LengthPercentageAuto::Px(13.0));
        assert_eq!(checkbox.box_sizing, BoxSizing::BorderBox);
        let button = control("button");
        assert_eq!(button.width, LengthPercentageAuto::Auto);
        assert_eq!(button.text_align, TextAlign::Center);
        assert_eq!(button.background_color, Color::rgb8(0xef, 0xef, 0xef));
        assert_eq!(control("select").width, input.width);
        assert_eq!(control("option").display, Display::None);
        let textarea = control("textarea");
        assert_eq!(textarea.height, LengthPercentageAuto::Px(2.4 * 13.3333));
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
        assert_eq!(parse_color(&css::tokenize("CornflowerBlue")), None); // not in the small set — documented gap
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
}
