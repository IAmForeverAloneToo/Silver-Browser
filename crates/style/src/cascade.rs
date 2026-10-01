//! The cascade: origin sorting, specificity, inheritance, and property
//! parsing — turning a `dom::Node` plus its stylesheets into a
//! [`StyledDom`] (roadmap M5).
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
//!   units (zero when none is given, and [`StyledDom::uses_viewport_units`]
//!   says whether any were met, so a caller can recascade on a viewport
//!   change only when it matters), and a [`FontMeasure`] for `ex` and
//!   `ch` (half an em each without one, the fallbacks css-values-4
//!   allows). Percentages for `width`, `height`, `margin` and `padding`
//!   are kept as written: what they are percentages of is the containing
//!   block's, which only layout knows. `font-size: <percentage>` is of the
//!   parent's font size and resolves here.
//! * `display`: `block` / `inline` / `inline-block` / `none` only
//!   (M5.1's own v1 scope, plus M10.4's `inline-block`).
//! * `float`, `clear`, `position`, `top`/`right`/`bottom`/`left` and
//!   `z-index` (M11), with CSS2 §9.7's computed-value rules applied
//!   after the longhands: an absolutely positioned box does not float,
//!   and an inline-level box that floats or is absolutely positioned
//!   computes to `block`.
//!
//! What every property accepts: the CSS-wide keywords `inherit`,
//! `initial` and `unset` (css-cascade-5 §7.3), resolved against the
//! parent's computed value or the property's initial value. What no
//! length may be: non-finite, or beyond [`MAX_LENGTH_PX`] in magnitude
//! (a `1e39px` would be infinity in `f32`, and infinity minus infinity
//! is the NaN that puts text at the top-left corner); `font-size`,
//! `padding` and `border-width` may not be negative either.
//!
//! The selectors of every rule are parsed once per sheet, not once per
//! element: a thousand rules against ten thousand elements is ten
//! million matches, which is the work, not ten million parses.

use std::cell::Cell;

use css::Token;
use dom::Node;

use crate::computed::{
    BOTTOM, BorderStyle, BoxSizing, Clear, Color, ComputedStyle, Direction, Display, Edges, Float,
    LEFT, LengthPercentage, LengthPercentageAuto, MarginEdges, Overflow, PaddingEdges, Position,
    RIGHT, TOP, TextAlign, TextTransform, WhiteSpace,
};
use crate::selector_matching::{
    FlatChild, FlatDom, Matcher, SelectorList, Specificity, parse_selector_list,
};

/// The built-in default stylesheet (M5.6), compiled in — loading it is
/// not resource fetching (no I/O at runtime), so it doesn't bend the
/// "parsers never fetch" rule.
const UA_STYLESHEET_SRC: &str = include_str!("../assets/ua.css");

/// A `FlatDom` with each node's [`ComputedStyle`] resolved alongside it:
/// `styles[i]` is the computed style for `flat.nodes[i]`, including
/// index 0 (the virtual document root — never itself painted, but real
/// top-level elements inherit from its always-initial-values style).
pub struct StyledDom {
    /// The structural view (also used for selector matching).
    pub flat: FlatDom,
    /// Parallel to `flat.nodes` — `styles[i]` is `flat.nodes[i]`'s style.
    pub styles: Vec<ComputedStyle>,
    /// Whether any declaration used a viewport unit (`vw`, `vh`, `vmin`,
    /// `vmax`), so that these styles depend on the viewport the cascade
    /// was given and must be recomputed when it changes (M10.1).
    pub uses_viewport_units: bool,
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

/// What the cascade resolves relative units against (M10.1).
#[derive(Clone, Copy, Default)]
pub struct CascadeContext<'a> {
    /// The viewport's size in CSS pixels, for `vw`, `vh`, `vmin` and
    /// `vmax`. Zero when unknown: those units then resolve to zero.
    pub viewport_width: f32,
    pub viewport_height: f32,
    /// The font measures for `ex` and `ch`, or `None` for the fallbacks.
    pub fonts: Option<&'a dyn FontMeasure>,
}

/// Resolves computed styles for every element in `document`, cascading
/// the built-in UA stylesheet (lowest precedence) under `author_sheets`
/// (in document order) and each element's inline `style` attribute
/// (highest precedence short of `!important` — see [`tier`]), with no
/// viewport and no font (see [`CascadeContext`]): what a test or a
/// fuzz target wants. The engine uses [`style_document_in`].
pub fn style_document(document: &Node, author_sheets: &[css::Stylesheet]) -> StyledDom {
    style_document_in(document, author_sheets, &CascadeContext::default())
}

/// [`style_document`] resolving the viewport and font units against
/// `context`.
pub fn style_document_in(
    document: &Node,
    author_sheets: &[css::Stylesheet],
    context: &CascadeContext<'_>,
) -> StyledDom {
    let ua = css::parse_stylesheet(UA_STYLESHEET_SRC)
        .expect("the built-in UA stylesheet (assets/ua.css) is valid CSS");
    style_document_with_ua(document, &ua, author_sheets, context)
}

/// [`style_document_in`] with the user-agent stylesheet given rather
/// than built in — what a test of the UA tiers needs, since
/// `assets/ua.css` has no `!important` of its own to assert against.
pub(crate) fn style_document_with_ua(
    document: &Node,
    ua: &css::Stylesheet,
    author_sheets: &[css::Stylesheet],
    context: &CascadeContext<'_>,
) -> StyledDom {
    let flat = FlatDom::build(document);
    let ua_rules = parse_rules(ua);
    let author_rules: Vec<Vec<ParsedRule<'_>>> = author_sheets.iter().map(parse_rules).collect();
    let mut matcher = Matcher::new(&flat);
    let mut styles = vec![ComputedStyle::default(); flat.nodes.len()];
    let viewport_used = Cell::new(false);
    // `rem` is the root element's font size: known once the root (the
    // first element under the document) is styled, and the initial
    // value for the root itself.
    let mut root_font_size: Option<f32> = None;
    // Preorder with an explicit stack: an element's children are styled
    // together, from its own finished style, before any grandchild is
    // reached, and no depth of tree costs stack.
    let mut pending = vec![0usize];
    while let Some(idx) = pending.pop() {
        let parent_style = styles[idx].clone();
        let child_element_indices: Vec<usize> = flat.nodes[idx]
            .children
            .iter()
            .filter_map(|c| match c {
                FlatChild::Element(i) => Some(*i),
                FlatChild::Text(_) => None,
            })
            .collect();
        for &child_idx in &child_element_indices {
            let applied =
                declarations_for(&flat, &mut matcher, child_idx, &ua_rules, &author_rules);
            let resolution = Resolution {
                rem: root_font_size.unwrap_or(ComputedStyle::default().font_size_px),
                context,
                viewport_used: &viewport_used,
            };
            styles[child_idx] = compute_style(
                &applied,
                &parent_style,
                flat.attr(child_idx, "dir"),
                &resolution,
            );
            if root_font_size.is_none() {
                root_font_size = Some(styles[child_idx].font_size_px);
            }
        }
        pending.extend(child_element_indices.into_iter().rev());
    }
    StyledDom {
        flat,
        styles,
        uses_viewport_units: viewport_used.get(),
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
}

/// The bases a length in one element's declarations resolves against:
/// `em` is the element's own font size, except for `font-size` itself,
/// where it is the parent's.
#[derive(Clone, Copy)]
struct LengthBases<'a> {
    em: f32,
    resolution: &'a Resolution<'a>,
}

/// A qualified rule with its selector list parsed, once, for the whole
/// pass over the document. A rule whose prelude does not parse is
/// dropped here, per css-syntax-3 (M4.10).
struct ParsedRule<'s> {
    list: SelectorList,
    declarations: &'s [css::Declaration],
}

fn parse_rules(sheet: &css::Stylesheet) -> Vec<ParsedRule<'_>> {
    sheet
        .rules
        .iter()
        .filter_map(|rule| match rule {
            css::Rule::Qualified(qr) => Some(ParsedRule {
                list: parse_selector_list(&qr.prelude).ok()?,
                declarations: &qr.declarations,
            }),
            // At-rules are skipped (see the module doc).
            css::Rule::At(_) => None,
        })
        .collect()
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

/// Every declaration that applies to element `idx`, tagged with its
/// cascade key — unsorted; [`compute_style`] sorts before applying.
fn declarations_for<'a>(
    flat: &FlatDom,
    matcher: &mut Matcher<'a>,
    idx: usize,
    ua: &'a [ParsedRule<'a>],
    author_sheets: &'a [Vec<ParsedRule<'a>>],
) -> Vec<Applied> {
    let mut order = 0u32;
    let mut out = Vec::new();
    collect_from_sheet(matcher, idx, ua, TierBase::UserAgent, &mut order, &mut out);
    for sheet in author_sheets {
        collect_from_sheet(
            matcher,
            idx,
            sheet,
            TierBase::AuthorSheet,
            &mut order,
            &mut out,
        );
    }
    if let Some(style_attr) = flat.attr(idx, "style")
        && let Ok(decls) = css::parse_style_attribute(style_attr)
    {
        for decl in decls {
            out.push(Applied {
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

/// Matches `sheet`'s rules against element `idx`, pushing one
/// [`Applied`] per declaration in every rule that matches.
fn collect_from_sheet<'a>(
    matcher: &mut Matcher<'a>,
    idx: usize,
    sheet: &'a [ParsedRule<'a>],
    base: TierBase,
    order: &mut u32,
    out: &mut Vec<Applied>,
) {
    for rule in sheet {
        // A comma-separated list is cascade-equivalent to separate rules
        // sharing one declaration block — each matching branch
        // contributes independently, so the specificity that matters is
        // the highest among the branches that actually matched here.
        let specificity = rule
            .list
            .0
            .iter()
            .filter(|s| matcher.matches(idx, s))
            .map(|s| s.specificity)
            .max();
        let Some(specificity) = specificity else {
            continue;
        };
        for decl in rule.declarations {
            out.push(Applied {
                tier: tier(base, decl.important),
                specificity,
                order: *order,
                decl: decl.clone(),
            });
            *order += 1;
        }
    }
}

/// Sorts `applied` into cascade order and folds it into a fresh
/// `ComputedStyle` inherited from `parent`.
fn compute_style(
    applied: &[Applied],
    parent: &ComputedStyle,
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
        apply_declaration(&mut style, &a.decl, &bases, parent, &mut border_color_set);
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
    // a float or an absolutely positioned box that is inline-level
    // becomes a block, so the box tree sees a block container. (The
    // root element's `float` is `none` too; the box tree builder keeps
    // the top-level boxes in flow.)
    if style.position.is_out_of_flow() {
        style.float = Float::None;
    }
    if (style.float != Float::None || style.position.is_out_of_flow())
        && matches!(style.display, Display::Inline | Display::InlineBlock)
    {
        style.display = Display::Block;
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
        _ => None, // flex/grid/table/... land with their milestones
    }
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
        };
        let viewport_used = Cell::new(false);
        let resolution = Resolution {
            rem: 16.0,
            context: &context,
            viewport_used: &viewport_used,
        };
        f(&LengthBases {
            em: 16.0,
            resolution: &resolution,
        })
    }

    /// Styles `html` and returns the computed style of the element found
    /// by `selector` (the first match, in document order).
    fn style_of(html: &str, sheets: &[css::Stylesheet], selector: &str) -> ComputedStyle {
        let document = html::parse_document(html).expect("html parsing never fails");
        let styled = style_document(&document, sheets);
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        styled
            .flat
            .element_indices()
            .find(|&i| matches_selector(&styled.flat, i, &list.0[0]))
            .map(|i| styled.styles[i].clone())
            .unwrap_or_else(|| panic!("no element matched {selector:?}"))
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
        // stands in through `style_document_with_ua`.
        let ua = sheet("p { color: red !important; display: block; }");
        let author = [sheet(
            "p { color: blue !important; display: inline !important; }",
        )];
        let document = html::parse_document("<p>x</p>").expect("html parsing never fails");
        let styled = style_document_with_ua(&document, &ua, &author, &CascadeContext::default());
        let p = styled
            .flat
            .element_indices()
            .find(|&i| styled.flat.nodes[i].tag == "p")
            .expect("a p");
        assert_eq!(styled.styles[p].color, Color::rgb8(255, 0, 0));
        // A normal UA declaration still loses to an important author one.
        assert_eq!(styled.styles[p].display, Display::Inline);
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
        let document = html::parse_document(html).expect("html parsing never fails");
        let styled = style_document_in(&document, sheets, context);
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        styled
            .flat
            .element_indices()
            .find(|&i| matches_selector_list(&styled.flat, i, &list))
            .map(|i| styled.styles[i].clone())
            .expect("the selector matches an element")
    }

    #[test]
    fn relative_units_resolve_against_the_context() {
        let context = CascadeContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            fonts: Some(&StubFont),
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
        let document = html::parse_document("<p>x</p>").expect("parses");
        let styled = style_document(&document, &sheets);
        assert!(styled.uses_viewport_units);
        let p = styled
            .flat
            .element_indices()
            .find(|&i| styled.flat.nodes[i].tag == "p")
            .expect("the p");
        assert_eq!(styled.styles[p].width, LengthPercentageAuto::Px(0.0));
        assert_eq!(styled.styles[p].margin[TOP], LengthPercentageAuto::Px(8.0));
        assert_eq!(
            styled.styles[p].margin[RIGHT],
            LengthPercentageAuto::Px(8.0)
        );
        // A document without viewport units says so, so a resize need
        // not recascade it.
        let sheets = [sheet("p { width: 10em; margin: 1rem }")];
        let styled = style_document(&document, &sheets);
        assert!(!styled.uses_viewport_units);
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
    fn display_none_is_recorded_but_children_still_get_styles() {
        let sheets = [sheet("div { display: none; } span { color: red; }")];
        let document = html::parse_document("<div><span>x</span></div>").unwrap();
        let styled = style_document(&document, &sheets);
        let index_of = |tag: &str| {
            styled
                .flat
                .element_indices()
                .find(|&i| styled.flat.nodes[i].tag == tag)
                .expect("the element exists")
        };
        let (div, span) = (index_of("div"), index_of("span"));
        assert_eq!(styled.styles[div].display, Display::None);
        // Styling doesn't itself decide what's painted — that's paint's
        // job, walking `display` — so a display:none subtree's children
        // still have well-defined styles.
        assert_eq!(styled.styles[span].color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn display_parses_its_four_keywords_and_is_not_inherited() {
        let sheets = [sheet("div { display: inline-block; }")];
        let document = html::parse_document("<div><span>x</span></div>").unwrap();
        let styled = style_document(&document, &sheets);
        let index_of = |tag: &str| {
            styled
                .flat
                .element_indices()
                .find(|&i| styled.flat.nodes[i].tag == tag)
                .expect("the element exists")
        };
        assert_eq!(styled.styles[index_of("div")].display, Display::InlineBlock);
        assert_eq!(styled.styles[index_of("span")].display, Display::Inline);
        let html = "<div><span>x</span></div>";
        for (value, expected) in [
            ("block", Display::Block),
            ("inline", Display::Inline),
            ("INLINE-BLOCK", Display::InlineBlock),
            ("none", Display::None),
        ] {
            let sheets = [sheet(&format!("span {{ display: {value}; }}"))];
            assert_eq!(style_of(html, &sheets, "span").display, expected, "{value}");
        }
        // An unknown keyword leaves the UA value in place.
        let sheets = [sheet("div { display: flex; }")];
        assert_eq!(style_of(html, &sheets, "div").display, Display::Block);
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
